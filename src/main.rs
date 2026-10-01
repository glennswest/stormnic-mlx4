//! stormnic-mlx4: an `EFI_SIMPLE_NETWORK_PROTOCOL` driver for ConnectX-3, in Rust.
//!
//! The image is an EFI boot-service driver (see `build.rs`). Its entry point
//! installs `EFI_DRIVER_BINDING_PROTOCOL` on its own image handle and returns;
//! the firmware's `ConnectController` (stormbootx runs one after loading the
//! media's drivers) then offers it every controller. The binding is written
//! here rather than taken from uefi-rs, whose `Stop` refuses child handles.
//!
//! `Supported` takes a PCI function whose vendor/device is a ConnectX-3
//! (15b3:1003) or ConnectX-3 Pro (15b3:1007) and that no other driver holds.
//! A platform's own driver always wins: a NIC that already carries an SNP, or
//! whose PCI I/O another driver has opened `BY_DRIVER`, is left alone.
//!
//! `Start` brings the firmware up (`fw.rs`) and every Ethernet port
//! (`eth.rs`), waits a few seconds for link, and installs an SNP with a MAC
//! device path on one child handle per Ethernet port (`snp.rs`). `Stop`
//! takes the children down, then the device (spec 6.4). At ExitBootServices
//! the device is stopped without freeing anything, so no DMA runs into
//! memory the OS takes over.
//!
//! Everything about the hardware comes from `docs/spec/connectx3.md`.
#![no_main]
#![no_std]

extern crate alloc;

mod bars;
mod dma;
mod eth;
mod fw;
mod hcr;
mod pci;
mod snp;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr;

use uefi::boot::{self, OpenProtocolAttributes, OpenProtocolParams, ScopedProtocol};
use uefi::prelude::*;
use uefi::proto::network::snp::SimpleNetwork;
use uefi::{Handle, Identify};
use uefi_raw::protocol::device_path::DevicePathProtocol;
use uefi_raw::protocol::driver::DriverBindingProtocol;
use uefi_raw::table::boot::{BootServices, EventType, Tpl};
use uefi_raw::Event;

use pci::PciIo;

const VENDOR_MELLANOX: u16 = 0x15b3;

/// The devices this driver takes, and what to call them in the log.
const DEVICES: &[(u16, &str)] = &[(0x1003, "ConnectX-3"), (0x1007, "ConnectX-3 Pro")];

/// `EFI_OPEN_PROTOCOL_*` attributes, for the raw `OpenProtocol` calls.
const OPEN_GET_PROTOCOL: u32 = 0x02;
pub const OPEN_BY_CHILD_CONTROLLER: u32 = 0x08;
const OPEN_BY_DRIVER: u32 = 0x10;

/// How long `Start` waits for link before installing the SNPs (5.8).
const LINK_WAIT_MS: u32 = 5_000;

fn model(vendor: u16, device: u16) -> Option<&'static str> {
    if vendor != VENDOR_MELLANOX {
        return None;
    }
    DEVICES.iter().find(|(d, _)| *d == device).map(|(_, n)| *n)
}

/// The boot services table, for the calls uefi-rs has no wrapper for.
pub fn bs() -> &'static BootServices {
    let st = uefi::table::system_table_raw().expect("system table");
    // SAFETY: the driver only runs while boot services exist (the
    // ExitBootServices notify function runs before they go).
    unsafe { &*(*st.as_ptr()).boot_services }
}

/// The TPL raised for a scope, restored when dropped.
pub struct TplGuard(Tpl);

impl TplGuard {
    pub fn raise(tpl: Tpl) -> TplGuard {
        // SAFETY: raising to TPL_CALLBACK is what every SNP driver does
        // around its hardware access.
        TplGuard(unsafe { (bs().raise_tpl)(tpl) })
    }
}

impl Drop for TplGuard {
    fn drop(&mut self) {
        unsafe { (bs().restore_tpl)(self.0) };
    }
}

/// Open the controller's PCI I/O. `GetProtocol` only looks; `ByDriver` claims
/// it, and fails with `ACCESS_DENIED` when another driver already has.
fn open_pci(
    agent: Handle,
    controller: Handle,
    attributes: OpenProtocolAttributes,
) -> uefi::Result<ScopedProtocol<PciIo>> {
    // SAFETY: the interface is only used for the duration of the callback,
    // and the firmware serialises driver-binding calls.
    unsafe {
        boot::open_protocol::<PciIo>(
            OpenProtocolParams { handle: controller, agent, controller: Some(controller) },
            attributes,
        )
    }
}

/// A ConnectX-3 on this controller: where it is and what it is. `None` for
/// anything else, which is every other handle in the system, so no logging.
fn identify(agent: Handle, controller: Handle) -> Option<(pci::Location, u16, &'static str)> {
    let mut pci = open_pci(agent, controller, OpenProtocolAttributes::GetProtocol).ok()?;
    let (vendor, device) = pci.ids().ok()?;
    let name = model(vendor, device)?;
    let loc = pci.location().ok()?;
    Some((loc, device, name))
}

/// A started controller: the device, its ports and their children.
pub struct Nic {
    controller: *mut c_void,
    pub pci: *mut PciIo,
    pub hca: fw::Hca,
    pub eth: eth::Eth,
    /// Boxed: each child's SNP address is what the firmware holds.
    #[allow(clippy::vec_box)]
    children: Vec<Box<snp::Child>>,
    ebs: Event,
    /// ExitBootServices has stopped the device.
    pub dead: bool,
}

/// The driver binding interface and the controllers it has started.
#[repr(C)]
struct Binding {
    proto: DriverBindingProtocol,
    nics: Vec<*mut Nic>,
}

/// # Safety
/// `this` is the interface `main` installed.
unsafe fn binding<'a>(this: *const DriverBindingProtocol) -> &'a mut Binding {
    unsafe { &mut *this.cast_mut().cast::<Binding>() }
}

unsafe extern "efiapi" fn supported(
    this: *const DriverBindingProtocol,
    controller: *mut c_void,
    _remaining: *const DevicePathProtocol,
) -> Status {
    let b = unsafe { binding(this) };
    let (Some(agent), Some(controller)) =
        (unsafe { Handle::from_ptr(b.proto.driver_binding_handle) }, unsafe { Handle::from_ptr(controller) })
    else {
        return Status::INVALID_PARAMETER;
    };
    let Some((loc, device, name)) = identify(agent, controller) else {
        return Status::UNSUPPORTED;
    };
    if b.nics.iter().any(|&n| unsafe { (*n).controller } == controller.as_ptr()) {
        return Status::ALREADY_STARTED;
    }
    let tag = || uefi::println!("stormnic-mlx4: {loc} {VENDOR_MELLANOX:04x}:{device:04x} {name}:");

    let snp = OpenProtocolParams { handle: controller, agent, controller: None };
    if boot::test_protocol::<SimpleNetwork>(snp).unwrap_or(false) {
        tag();
        uefi::println!("  Supported: already has an SNP, leaving it to the platform's driver");
        return Status::UNSUPPORTED;
    }

    // Claim and immediately release: Supported must leave no trace.
    match open_pci(agent, controller, OpenProtocolAttributes::ByDriver) {
        Ok(_pci) => {
            tag();
            uefi::println!("  Supported: yes");
            Status::SUCCESS
        }
        Err(e) => {
            tag();
            uefi::println!("  Supported: no, PCI I/O is held ({:?})", e.status());
            e.status()
        }
    }
}

unsafe extern "efiapi" fn start(
    this: *const DriverBindingProtocol,
    controller: *mut c_void,
    _remaining: *const DevicePathProtocol,
) -> Status {
    let b = unsafe { binding(this) };
    let agent = b.proto.driver_binding_handle;
    let bs = bs();
    let mut iface: *mut c_void = ptr::null_mut();
    let st = unsafe { (bs.open_protocol)(controller, &PciIo::GUID, &mut iface, agent, controller, OPEN_BY_DRIVER) };
    if st.is_error() {
        uefi::println!("stormnic-mlx4: Start: cannot claim PCI I/O ({st:?})");
        return st;
    }
    let close_pci = || {
        let _ = unsafe { (bs.close_protocol)(controller, &PciIo::GUID, agent, controller) };
    };
    let pci_ptr = iface.cast::<PciIo>();
    // SAFETY: opened BY_DRIVER above; it stays open until `stop`.
    let pci = unsafe { &mut *pci_ptr };
    let (Ok((vendor, device)), Ok(loc)) = (pci.ids(), pci.location()) else {
        close_pci();
        return Status::DEVICE_ERROR;
    };
    let name = model(vendor, device).unwrap_or("?");
    uefi::println!("stormnic-mlx4: {loc} {vendor:04x}:{device:04x} {name}:");
    uefi::println!("  Start: bound; bringing up the firmware");

    let mut path: *mut c_void = ptr::null_mut();
    let st = unsafe {
        (bs.open_protocol)(controller, &DevicePathProtocol::GUID, &mut path, agent, controller, OPEN_GET_PROTOCOL)
    };
    if st.is_error() {
        uefi::println!("  Start: the controller has no device path ({st:?}); releasing the NIC");
        close_pci();
        return Status::UNSUPPORTED;
    }

    let Ok((mut hca, setup)) = fw::open(pci) else {
        uefi::println!("  Start: firmware bring-up failed, releasing the NIC");
        close_pci();
        return Status::DEVICE_ERROR;
    };
    let mut eth = match eth::open(&mut hca, pci, &setup) {
        Ok(eth) if !eth.ports.is_empty() => eth,
        r => {
            match r {
                Ok(_) => uefi::println!("  Start: no Ethernet port, releasing the NIC"),
                Err(_) => uefi::println!("  Start: data path bring-up failed, releasing the NIC"),
            }
            fw::close(hca, pci);
            close_pci();
            return if r.is_ok() { Status::UNSUPPORTED } else { Status::DEVICE_ERROR };
        }
    };
    eth.wait_link(&mut hca, pci, LINK_WAIT_MS);

    let nic = Box::into_raw(Box::new(Nic {
        controller,
        pci: pci_ptr,
        hca,
        eth,
        children: Vec::new(),
        ebs: ptr::null_mut(),
        dead: false,
    }));
    // SAFETY: `nic` is leaked until `stop` takes it back.
    let n = unsafe { &mut *nic };
    for i in 0..n.eth.ports.len() {
        match unsafe { snp::Child::install(nic, i, path.cast(), agent, controller) } {
            Ok(c) => n.children.push(c),
            Err(_) => uefi::println!("  port {}: no SNP", n.eth.ports[i].num()),
        }
    }
    if n.children.is_empty() {
        uefi::println!("  Start: no SNP installed, releasing the NIC");
        let n = *unsafe { Box::from_raw(nic) };
        fw::close(n.hca, unsafe { &mut *n.pci });
        close_pci();
        return Status::DEVICE_ERROR;
    }
    let st = unsafe {
        (bs.create_event)(EventType::SIGNAL_EXIT_BOOT_SERVICES, Tpl::NOTIFY, Some(exit_boot_services), nic.cast(), &mut n.ebs)
    };
    if st.is_error() {
        // Without it the device would keep DMA running into the OS's memory.
        uefi::println!("  Start: ExitBootServices event: {st:?}; releasing the NIC");
        for c in n.children.iter_mut() {
            c.uninstall(agent, controller);
        }
        let n = *unsafe { Box::from_raw(nic) };
        fw::close(n.hca, unsafe { &mut *n.pci });
        close_pci();
        return Status::DEVICE_ERROR;
    }
    b.nics.push(nic);
    uefi::println!("  Start: {} SNP child handle(s) installed", n.children.len());
    Status::SUCCESS
}

unsafe extern "efiapi" fn stop(
    this: *const DriverBindingProtocol,
    controller: *mut c_void,
    children: usize,
    child_handles: *const *mut c_void,
) -> Status {
    let b = unsafe { binding(this) };
    let agent = b.proto.driver_binding_handle;
    let Some(at) = b.nics.iter().position(|&n| unsafe { (*n).controller } == controller) else {
        return Status::DEVICE_ERROR;
    };
    let nic = b.nics[at];
    let n = unsafe { &mut *nic };

    if children != 0 {
        // SAFETY: the firmware passes `children` handles.
        let handles = unsafe { core::slice::from_raw_parts(child_handles, children) };
        let mut ok = true;
        for &h in handles {
            if let Some(i) = n.children.iter().position(|c| c.handle == h) {
                if n.children[i].uninstall(agent, controller) {
                    n.children.remove(i);
                } else {
                    ok = false;
                }
            }
        }
        uefi::println!("stormnic-mlx4: Stop: {} child handle(s) left", n.children.len());
        return if ok { Status::SUCCESS } else { Status::DEVICE_ERROR };
    }
    if !n.children.is_empty() {
        return Status::DEVICE_ERROR;
    }
    let bs = bs();
    let _ = unsafe { (bs.close_event)(n.ebs) };
    b.nics.remove(at);
    let n = *unsafe { Box::from_raw(nic) };
    uefi::println!("stormnic-mlx4: Stop: tearing the device down");
    if !n.dead {
        fw::close(n.hca, unsafe { &mut *n.pci });
    }
    let _ = unsafe { (bs.close_protocol)(controller, &PciIo::GUID, agent, controller) };
    Status::SUCCESS
}

/// ExitBootServices: stop the device's DMA (6.4). No memory services, no
/// console output.
unsafe extern "efiapi" fn exit_boot_services(_event: Event, ctx: *mut c_void) {
    // SAFETY: the context is a started NIC, whose event `stop` closes first.
    let n = unsafe { &mut *ctx.cast::<Nic>() };
    if !n.dead {
        fw::quiesce(&mut n.hca, unsafe { &mut *n.pci });
        n.dead = true;
    }
}

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();
    let version = env!("CARGO_PKG_VERSION");
    let image = boot::image_handle().as_ptr();
    let b = Box::into_raw(Box::new(Binding {
        proto: DriverBindingProtocol {
            supported,
            start,
            stop,
            version: 0x10,
            image_handle: image,
            driver_binding_handle: image,
        },
        nics: Vec::new(),
    }));
    // SAFETY: the interface is leaked, so it lives as long as the image;
    // `build.rs` links the image as a boot-service driver, so its code stays.
    let r = unsafe {
        boot::install_protocol_interface(Some(boot::image_handle()), &DriverBindingProtocol::GUID, b.cast::<c_void>().cast_const())
    };
    match r {
        Ok(_) => {
            uefi::println!(
                "stormnic-mlx4 {version}: driver binding installed (15b3:1003 ConnectX-3, 15b3:1007 ConnectX-3 Pro)"
            );
            Status::SUCCESS
        }
        Err(e) => {
            uefi::println!("stormnic-mlx4 {version}: driver binding not installed: {:?}", e.status());
            e.status()
        }
    }
}
