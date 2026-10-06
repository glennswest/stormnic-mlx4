//! `EFI_SIMPLE_NETWORK_PROTOCOL` on a child handle per Ethernet port, with a
//! MAC device path (UEFI spec, "Simple Network Protocol"; the data path
//! behind it is docs/spec/connectx3.md 6.2 and 6.3).
//!
//! Every function raises the TPL to `TPL_CALLBACK`, so SNP calls, the
//! `WaitForPacket` notify function and the firmware's timer callbacks never
//! run inside one another. After ExitBootServices every call returns
//! `DEVICE_ERROR`.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr;

use uefi::Status;
use uefi_raw::protocol::device_path::DevicePathProtocol;
use uefi_raw::protocol::network::snp::{
    InterruptStatus, NetworkMode, NetworkState, NetworkStatistics, ReceiveFlags, SimpleNetworkProtocol,
};
use uefi_raw::table::boot::{EventType, InterfaceType, Tpl};
use uefi_raw::{Boolean, Event, Handle, IpAddress, MacAddress};

use crate::eth::{self, Mac, Send};
use crate::pci::PciIo;
use uefi::Identify;
use crate::{bs, Nic, TplGuard, OPEN_BY_CHILD_CONTROLLER};

/// `EFI_SIMPLE_NETWORK_PROTOCOL_REVISION`.
const REVISION: u64 = 0x0001_0000;
/// Multicast addresses the filter holds (the mode structure has room for 16).
const MAX_MCAST: usize = 16;
/// Filters implemented: unicast to the port MAC, broadcast, listed multicast.
const FILTERS: u32 = ReceiveFlags::UNICAST.bits() | ReceiveFlags::MULTICAST.bits() | ReceiveFlags::BROADCAST.bits();
/// Frames logged one line each, per direction and port.
const LOG_FRAMES: u32 = 16;
/// Without port-change events, `GetStatus` asks QUERY_PORT this often (in calls).
const LINK_POLL_CALLS: u32 = 64;
/// Messaging device path, MAC address node (UEFI spec, "MAC Address Device Path").
const MSG: u8 = 3;
const MSG_MAC: u8 = 11;
const MAC_NODE_LEN: usize = 37;
/// `IfType` for Ethernet (RFC 3232, ARP hardware type 1).
const IF_ETHERNET: u8 = 1;

/// One port's SNP. `snp` comes first so the `this` pointer every SNP
/// function receives is also a pointer to the child.
#[repr(C)]
pub struct Child {
    snp: SimpleNetworkProtocol,
    mode: NetworkMode,
    nic: *mut Nic,
    port: usize,
    pub handle: Handle,
    path: Vec<u8>,
    rx_logged: u32,
    tx_logged: u32,
    /// An own frame looped back has been logged (spec 7 item 14, #21).
    own_logged: bool,
    status_calls: u32,
}

fn mac32(mac: [u8; 6]) -> MacAddress {
    let mut a = [0u8; 32];
    a[..6].copy_from_slice(&mac);
    MacAddress(a)
}

fn mac6(a: &MacAddress) -> [u8; 6] {
    a.0[..6].try_into().unwrap()
}

/// The parent's device path, its end node replaced by a MAC node and a new end.
fn child_path(parent: *const DevicePathProtocol, mac: [u8; 6]) -> Vec<u8> {
    let mut path = Vec::new();
    let mut p = parent.cast::<u8>();
    // SAFETY: a device path is a list of nodes, each giving its own length,
    // ending with the end-of-path node (type 0x7f, subtype 0xff).
    unsafe {
        loop {
            let (kind, sub) = (*p, *p.add(1));
            let len = usize::from(u16::from_le_bytes([*p.add(2), *p.add(3)]));
            if (kind == 0x7f && sub == 0xff) || len < 4 {
                break;
            }
            path.extend_from_slice(core::slice::from_raw_parts(p, len));
            p = p.add(len);
        }
    }
    let mut node = [0u8; MAC_NODE_LEN];
    node[0] = MSG;
    node[1] = MSG_MAC;
    node[2..4].copy_from_slice(&(MAC_NODE_LEN as u16).to_le_bytes());
    node[4..10].copy_from_slice(&mac);
    node[36] = IF_ETHERNET;
    path.extend_from_slice(&node);
    path.extend_from_slice(&[0x7f, 0xff, 4, 0]);
    path
}

impl Child {
    /// A child handle for port `port` of `nic`: device path and SNP
    /// installed, the parent's PCI I/O opened `BY_CHILD_CONTROLLER`.
    ///
    /// # Safety
    /// `nic` outlives the child; `parent` is the controller's device path.
    pub unsafe fn install(
        nic: *mut Nic,
        port: usize,
        parent: *const DevicePathProtocol,
        agent: Handle,
        controller: Handle,
    ) -> Result<Box<Child>, Status> {
        let n = unsafe { &mut *nic };
        let p = &n.eth.ports[port];
        let (num, mac, link) = (p.num(), p.mac(), p.link_up());
        let mut c = Box::new(Child {
            snp: SimpleNetworkProtocol {
                revision: REVISION,
                start,
                stop,
                initialize,
                reset,
                shutdown,
                receive_filters,
                station_address,
                statistics,
                multicast_ip_to_mac,
                non_volatile_data,
                get_status,
                transmit,
                receive,
                wait_for_packet: ptr::null_mut(),
                mode: ptr::null_mut(),
            },
            mode: NetworkMode {
                state: NetworkState::STOPPED,
                hw_address_size: 6,
                media_header_size: 14,
                max_packet_size: 1500,
                nv_ram_size: 0,
                nv_ram_access_size: 0,
                receive_filter_mask: FILTERS,
                receive_filter_setting: 0,
                max_mcast_filter_count: MAX_MCAST as u32,
                mcast_filter_count: 0,
                mcast_filter: [MacAddress([0; 32]); 16],
                current_address: mac32(mac),
                broadcast_address: mac32([0xff; 6]),
                permanent_address: mac32(mac),
                if_type: IF_ETHERNET,
                mac_address_changeable: Boolean::FALSE,
                multiple_tx_supported: Boolean::TRUE,
                media_present_supported: Boolean::TRUE,
                media_present: link.into(),
            },
            nic,
            port,
            handle: ptr::null_mut(),
            path: child_path(parent, mac),
            rx_logged: 0,
            tx_logged: 0,
            own_logged: false,
            status_calls: 0,
        });
        c.snp.mode = &raw mut c.mode;
        let ctx: *mut Child = &raw mut *c;

        let bs = bs();
        let mut ev: Event = ptr::null_mut();
        // SAFETY: the child is boxed and lives until `uninstall` closes the event.
        let st = unsafe { (bs.create_event)(EventType::NOTIFY_WAIT, Tpl::CALLBACK, Some(wait_notify), ctx.cast(), &mut ev) };
        if st.is_error() {
            uefi::println!("  port {num}: WaitForPacket event: {st:?}");
            return Err(st);
        }
        c.snp.wait_for_packet = ev;

        let mut h: Handle = ptr::null_mut();
        let path_guid = DevicePathProtocol::GUID;
        let snp_guid = SimpleNetworkProtocol::GUID;
        let st = unsafe {
            (bs.install_protocol_interface)(&mut h, &path_guid, InterfaceType::NATIVE_INTERFACE, c.path.as_ptr().cast())
        };
        if st.is_error() {
            uefi::println!("  port {num}: device path install: {st:?}");
            let _ = unsafe { (bs.close_event)(ev) };
            return Err(st);
        }
        let st = unsafe { (bs.install_protocol_interface)(&mut h, &snp_guid, InterfaceType::NATIVE_INTERFACE, (&raw const c.snp).cast()) };
        if st.is_error() {
            uefi::println!("  port {num}: SNP install: {st:?}");
            unsafe {
                let _ = (bs.uninstall_protocol_interface)(h, &path_guid, c.path.as_ptr().cast());
                let _ = (bs.close_event)(ev);
            }
            return Err(st);
        }
        c.handle = h;
        let mut iface: *mut c_void = ptr::null_mut();
        let st = unsafe { (bs.open_protocol)(controller, &PciIo::GUID, &mut iface, agent, h, OPEN_BY_CHILD_CONTROLLER) };
        if st.is_error() {
            uefi::println!("  port {num}: PCI I/O by child: {st:?}");
        }
        uefi::println!(
            "  port {num}: SNP installed on a child handle, MAC {}, media {}",
            Mac(mac),
            if link { "present" } else { "absent" }
        );
        Ok(c)
    }

    /// Take the child down: close the parent's PCI I/O, uninstall the SNP
    /// and the device path. False (and the child left as it was) when the
    /// firmware refuses, e.g. because MNP will not let go of the SNP.
    pub fn uninstall(&mut self, agent: Handle, controller: Handle) -> bool {
        let bs = bs();
        let (path_guid, snp_guid) = (DevicePathProtocol::GUID, SimpleNetworkProtocol::GUID);
        unsafe {
            let _ = (bs.close_protocol)(controller, &PciIo::GUID, agent, self.handle);
            let st = (bs.uninstall_protocol_interface)(self.handle, &snp_guid, (&raw const self.snp).cast());
            if st.is_error() {
                let mut iface: *mut c_void = ptr::null_mut();
                let _ = (bs.open_protocol)(controller, &PciIo::GUID, &mut iface, agent, self.handle, OPEN_BY_CHILD_CONTROLLER);
                uefi::println!("stormnic-mlx4: port child: SNP uninstall refused ({st:?})");
                return false;
            }
            let _ = (bs.uninstall_protocol_interface)(self.handle, &path_guid, self.path.as_ptr().cast());
            let _ = (bs.close_event)(self.snp.wait_for_packet);
        }
        true
    }

    fn port(&mut self) -> &mut eth::Port {
        // SAFETY: the NIC outlives its children.
        let n = unsafe { &mut *self.nic };
        &mut n.eth.ports[self.port]
    }

    fn num(&mut self) -> u8 {
        self.port().num()
    }

    /// Poll the EQ and refresh `MediaPresent` (5.8).
    fn refresh(&mut self) {
        // SAFETY: the NIC outlives its children, and TPL_CALLBACK keeps
        // every other user of it out.
        let n = unsafe { &mut *self.nic };
        let pci = unsafe { &mut *n.pci };
        n.eth.poll(pci);
        n.eth.report_news(&mut n.hca, pci);
        self.status_calls = self.status_calls.wrapping_add(1);
        if !n.eth.events() && self.status_calls % LINK_POLL_CALLS == 1 {
            let _ = n.eth.refresh_link(&mut n.hca, pci, self.port);
        }
        self.mode.media_present = n.eth.ports[self.port].link_up().into();
    }

    fn log_frame(&mut self, dir: &str, f: &[u8], len: usize) {
        let logged = if dir == "rx" { &mut self.rx_logged } else { &mut self.tx_logged };
        if *logged >= LOG_FRAMES {
            return;
        }
        *logged += 1;
        let last = *logged == LOG_FRAMES;
        let num = self.num();
        uefi::println!(
            "stormnic-mlx4: port {num} {dir}: {len} bytes {} <- {} type {:04x}{}",
            Mac(f[0..6].try_into().unwrap()),
            Mac(f[6..12].try_into().unwrap()),
            u16::from_be_bytes([f[12], f[13]]),
            if last { " (no more logged)" } else { "" }
        );
    }

    /// Should a frame to `dst` from `src` reach the caller? Own frames looped
    /// back by the adapter never do (5.10).
    fn wanted(&mut self, dst: [u8; 6], src: [u8; 6], len: usize) -> bool {
        let mac = self.port().mac();
        let on = |f: ReceiveFlags| self.mode.receive_filter_setting & f.bits() != 0;
        if src == mac {
            // Spec 7 item 14: the RX QP's loopback source check should keep
            // these out; say so once if one gets through.
            if !core::mem::replace(&mut self.own_logged, true) {
                uefi::println!(
                    "stormnic-mlx4: port {} rx: own frame looped back ({len} bytes to {}), dropped (5.10); no more logged",
                    self.num(),
                    Mac(dst)
                );
            }
            false
        } else if dst == mac {
            on(ReceiveFlags::UNICAST)
        } else if dst == [0xff; 6] {
            on(ReceiveFlags::BROADCAST)
        } else if dst[0] & 1 != 0 {
            on(ReceiveFlags::MULTICAST)
                && self.mode.mcast_filter[..self.mode.mcast_filter_count as usize].iter().any(|m| mac6(m) == dst)
        } else {
            false
        }
    }
}

/// The child behind `this`, with the TPL raised; `None` after
/// ExitBootServices.
///
/// # Safety
/// `this` is an SNP this driver installed.
unsafe fn enter<'a>(this: *const SimpleNetworkProtocol) -> Option<(&'a mut Child, TplGuard)> {
    let guard = TplGuard::raise(Tpl::CALLBACK);
    let c = unsafe { &mut *this.cast_mut().cast::<Child>() };
    if unsafe { (*c.nic).dead } {
        return None;
    }
    Some((c, guard))
}

/// `NOT_STARTED` if stopped, `DEVICE_ERROR` if started but not initialised.
fn not_initialized(c: &Child) -> Status {
    if c.mode.state == NetworkState::STOPPED {
        Status::NOT_STARTED
    } else {
        Status::DEVICE_ERROR
    }
}

macro_rules! enter {
    ($this:expr) => {
        match unsafe { enter($this) } {
            Some(c) => c,
            None => return Status::DEVICE_ERROR,
        }
    };
}

macro_rules! initialized {
    ($c:expr) => {
        if $c.mode.state != NetworkState::INITIALIZED {
            return not_initialized($c);
        }
    };
}

unsafe extern "efiapi" fn start(this: *const SimpleNetworkProtocol) -> Status {
    let (c, _tpl) = enter!(this);
    if c.mode.state != NetworkState::STOPPED {
        return Status::ALREADY_STARTED;
    }
    c.mode.state = NetworkState::STARTED;
    uefi::println!("stormnic-mlx4: port {} SNP: started", c.num());
    Status::SUCCESS
}

unsafe extern "efiapi" fn stop(this: *const SimpleNetworkProtocol) -> Status {
    let (c, _tpl) = enter!(this);
    match c.mode.state {
        NetworkState::STARTED => {
            c.mode.state = NetworkState::STOPPED;
            uefi::println!("stormnic-mlx4: port {} SNP: stopped", c.num());
            Status::SUCCESS
        }
        NetworkState::STOPPED => Status::NOT_STARTED,
        _ => Status::DEVICE_ERROR,
    }
}

unsafe extern "efiapi" fn initialize(this: *const SimpleNetworkProtocol, _rx_extra: usize, _tx_extra: usize) -> Status {
    let (c, _tpl) = enter!(this);
    if c.mode.state != NetworkState::STARTED {
        return if c.mode.state == NetworkState::STOPPED { Status::NOT_STARTED } else { Status::DEVICE_ERROR };
    }
    // Frames that arrived before anyone listened are stale: drop them.
    while c.port().peek_rx().is_some() {
        c.port().pop_rx();
    }
    // Unicast and broadcast from the start; a caller that wants otherwise
    // says so with ReceiveFilters.
    c.mode.receive_filter_setting = ReceiveFlags::UNICAST.bits() | ReceiveFlags::BROADCAST.bits();
    c.mode.mcast_filter_count = 0;
    c.refresh();
    c.mode.state = NetworkState::INITIALIZED;
    let media = if bool::from(c.mode.media_present) { "present" } else { "absent" };
    uefi::println!("stormnic-mlx4: port {} SNP: initialized, media {media}", c.num());
    Status::SUCCESS
}

unsafe extern "efiapi" fn reset(this: *const SimpleNetworkProtocol, _extended: Boolean) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    // The queues run continuously; there is nothing to reinitialise.
    Status::SUCCESS
}

unsafe extern "efiapi" fn shutdown(this: *const SimpleNetworkProtocol) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    c.mode.receive_filter_setting = 0;
    c.mode.mcast_filter_count = 0;
    c.mode.state = NetworkState::STARTED;
    uefi::println!("stormnic-mlx4: port {} SNP: shut down", c.num());
    Status::SUCCESS
}

unsafe extern "efiapi" fn receive_filters(
    this: *const SimpleNetworkProtocol,
    enable: ReceiveFlags,
    disable: ReceiveFlags,
    reset_mcast: Boolean,
    count: usize,
    list: *const MacAddress,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    let reset_mcast = bool::from(reset_mcast);
    if enable.bits() & !FILTERS != 0 {
        return Status::INVALID_PARAMETER;
    }
    let mut new = Vec::new();
    if !reset_mcast && count != 0 {
        if count > MAX_MCAST || list.is_null() {
            return Status::INVALID_PARAMETER;
        }
        // SAFETY: the caller passes `count` addresses.
        let l = unsafe { core::slice::from_raw_parts(list, count) };
        if l.iter().any(|m| m.0[0] & 1 == 0) {
            return Status::INVALID_PARAMETER;
        }
        new.extend(l.iter().map(mac6));
    }
    // Disabling a filter that is not supported disables nothing.
    let setting = (c.mode.receive_filter_setting | enable.bits()) & !disable.bits();
    let n = unsafe { &mut *c.nic };
    let pci = unsafe { &mut *n.pci };
    for &m in &new {
        if n.eth.join(&mut n.hca, pci, c.port, m).is_err() {
            return Status::DEVICE_ERROR;
        }
    }
    if reset_mcast {
        c.mode.mcast_filter_count = 0;
    } else if !new.is_empty() {
        for (i, &m) in new.iter().enumerate() {
            c.mode.mcast_filter[i] = mac32(m);
        }
        c.mode.mcast_filter_count = new.len() as u32;
    }
    if setting != c.mode.receive_filter_setting || !new.is_empty() {
        uefi::println!(
            "stormnic-mlx4: port {} SNP: receive filters {setting:#x}, {} multicast address(es)",
            c.num(),
            c.mode.mcast_filter_count
        );
    }
    c.mode.receive_filter_setting = setting;
    Status::SUCCESS
}

unsafe extern "efiapi" fn station_address(this: *const SimpleNetworkProtocol, _reset: Boolean, _new: *const MacAddress) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    // MacAddressChangeable is FALSE.
    Status::UNSUPPORTED
}

unsafe extern "efiapi" fn statistics(
    this: *const SimpleNetworkProtocol,
    _reset: Boolean,
    _size: *mut usize,
    _table: *mut NetworkStatistics,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    Status::UNSUPPORTED
}

unsafe extern "efiapi" fn multicast_ip_to_mac(
    this: *const SimpleNetworkProtocol,
    ipv6: Boolean,
    ip: *const IpAddress,
    mac: *mut MacAddress,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    if ip.is_null() || mac.is_null() {
        return Status::INVALID_PARAMETER;
    }
    // SAFETY: an EFI_IP_ADDRESS is 16 bytes, an IPv4 address in the first 4.
    let a = unsafe { *ip.cast::<[u8; 16]>() };
    // RFC 1112 6.4 and RFC 2464 7.
    let m = if bool::from(ipv6) {
        [0x33, 0x33, a[12], a[13], a[14], a[15]]
    } else {
        if a[0] & 0xf0 != 0xe0 {
            return Status::INVALID_PARAMETER;
        }
        [0x01, 0x00, 0x5e, a[1] & 0x7f, a[2], a[3]]
    };
    unsafe { *mac = mac32(m) };
    Status::SUCCESS
}

unsafe extern "efiapi" fn non_volatile_data(
    this: *const SimpleNetworkProtocol,
    _read: Boolean,
    _offset: usize,
    _size: usize,
    _buffer: *mut c_void,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    Status::UNSUPPORTED
}

unsafe extern "efiapi" fn get_status(
    this: *const SimpleNetworkProtocol,
    irq: *mut InterruptStatus,
    tx_buf: *mut *mut c_void,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    c.refresh();
    if !irq.is_null() {
        let mut s = InterruptStatus::empty();
        if c.port().rx_ready() {
            s |= InterruptStatus::RECEIVE;
        }
        if c.port().sent_pending() {
            s |= InterruptStatus::TRANSMIT;
        }
        unsafe { *irq = s };
    }
    if !tx_buf.is_null() {
        let b = c.port().take_sent().map_or(ptr::null_mut(), |t| t as *mut c_void);
        unsafe { *tx_buf = b };
    }
    Status::SUCCESS
}

unsafe extern "efiapi" fn transmit(
    this: *const SimpleNetworkProtocol,
    header_size: usize,
    size: usize,
    buffer: *const c_void,
    src: *const MacAddress,
    dst: *const MacAddress,
    protocol: *const u16,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    if buffer.is_null() {
        return Status::INVALID_PARAMETER;
    }
    if header_size != 0 && (header_size != 14 || dst.is_null() || protocol.is_null()) {
        return Status::INVALID_PARAMETER;
    }
    if size < 14 {
        return Status::BUFFER_TOO_SMALL;
    }
    if size > eth::MAX_FRAME {
        return Status::INVALID_PARAMETER;
    }
    let mut f = [0u8; eth::MAX_FRAME];
    // SAFETY: the caller passes `size` bytes.
    f[..size].copy_from_slice(unsafe { core::slice::from_raw_parts(buffer.cast::<u8>(), size) });
    if header_size != 0 {
        let from = if src.is_null() { c.port().mac() } else { mac6(unsafe { &*src }) };
        f[0..6].copy_from_slice(&mac6(unsafe { &*dst }));
        f[6..12].copy_from_slice(&from);
        f[12..14].copy_from_slice(&unsafe { *protocol }.to_be_bytes());
    }
    let n = unsafe { &mut *c.nic };
    let pci = unsafe { &mut *n.pci };
    match n.eth.ports[c.port].send(pci, &f[..size], buffer as usize) {
        Ok(()) => {
            c.log_frame("tx", &f, size);
            Status::SUCCESS
        }
        Err(Send::Full) => Status::NOT_READY,
        Err(Send::TooLong) => Status::INVALID_PARAMETER,
        Err(Send::Broken) => Status::DEVICE_ERROR,
    }
}

unsafe extern "efiapi" fn receive(
    this: *const SimpleNetworkProtocol,
    header_size: *mut usize,
    size: *mut usize,
    buffer: *mut c_void,
    src: *mut MacAddress,
    dst: *mut MacAddress,
    protocol: *mut u16,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    if size.is_null() || buffer.is_null() {
        return Status::INVALID_PARAMETER;
    }
    loop {
        let Some((mem, len)) = c.port().peek_rx() else {
            return Status::NOT_READY;
        };
        let mut h = [0u8; 14];
        if len < h.len() {
            c.port().pop_rx();
            continue;
        }
        mem.read(0, &mut h);
        let (d, s): ([u8; 6], [u8; 6]) = (h[0..6].try_into().unwrap(), h[6..12].try_into().unwrap());
        if !c.wanted(d, s, len) {
            c.port().pop_rx();
            continue;
        }
        // SAFETY: the caller's buffer holds `*size` bytes.
        unsafe {
            if *size < len {
                *size = len;
                return Status::BUFFER_TOO_SMALL;
            }
            mem.read(0, core::slice::from_raw_parts_mut(buffer.cast::<u8>(), len));
            *size = len;
            if !header_size.is_null() {
                *header_size = 14;
            }
            if !src.is_null() {
                *src = mac32(s);
            }
            if !dst.is_null() {
                *dst = mac32(d);
            }
            if !protocol.is_null() {
                *protocol = u16::from_be_bytes([h[12], h[13]]);
            }
        }
        c.port().pop_rx();
        c.log_frame("rx", &h, len);
        return Status::SUCCESS;
    }
}

/// `WaitForPacket`'s notify function: signal the event when a frame waits.
unsafe extern "efiapi" fn wait_notify(event: Event, ctx: *mut c_void) {
    // SAFETY: the context is the child, which closes this event before it goes.
    let c = unsafe { &mut *ctx.cast::<Child>() };
    if unsafe { (*c.nic).dead } {
        return;
    }
    if c.mode.state == NetworkState::INITIALIZED && c.port().rx_ready() {
        let _ = unsafe { (bs().signal_event)(event) };
    }
}
