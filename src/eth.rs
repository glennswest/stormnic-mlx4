//! The Ethernet data path (#3): the objects one port needs, polling send and
//! receive, and a check that a broadcast frame makes a round trip
//! (docs/spec/connectx3.md sections 4–6; section numbers in comments are
//! that document's).
//!
//! `run` creates the shared objects in Linux's order (EQ and MAP_EQ,
//! CONF_SPECIAL_QP, the physical memory region), then takes each Ethernet
//! port in turn: CQs, QPs, SET_PORT, INIT_PORT and steering (6.1 steps
//! 15–27); waits for link; broadcasts a DHCPDISCOVER (and, once an address
//! has been seen on the wire, an ARP probe for it); logs what comes back; and
//! takes the port down again. Every object created pushes the command that
//! takes it back onto the `Hca` undo stack, so teardown runs 6.4's order.
//! Object numbers are per port (4.1), so #4 can keep both ports up at once.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{fence, Ordering};
use core::time::Duration;

use uefi::boot;

use crate::dma::{Mem, PAGE};
use crate::fw::{self, fail, Fail, Hca, PortInfo, Setup, BAR_UAR};
use crate::hcr;
use crate::pci::PciIo;

/// Queue sizes: EQ entries (4.6), receive descriptors (5.6), send TXBBs (5.7).
const EQ_ENTRIES: u32 = 256;
const RX_ENTRIES: u32 = 256;
const TX_TXBBS: u32 = 128;
/// Entry sizes: EQE and CQE (32 bytes, 4.6/4.7), TXBB (5.7), receive stride (5.6).
const EQE: usize = 32;
const CQE: usize = 32;
const TXBB: usize = 64;
const RX_STRIDE: usize = 16;
/// One receive buffer per descriptor, one bounce buffer per TXBB (5.6).
const BUF: usize = 2048;
/// TXBBs the device may prefetch past the producer (5.7).
const HEADROOM: u32 = 33;
/// SET_PORT frame size: MTU 1500 + 14 + 8 + 4 (5.9).
const PORT_MTU: u16 = 1526;
/// Shortest frame sent; the device would pad, padding here is harmless (5.7).
const MIN_FRAME: usize = 60;
/// Events mapped to the EQ: CQ error, WQ catastrophic, local catastrophic,
/// port change, WQ invalid request, WQ access error; not command completion (4.6).
const EVENT_MASK: u64 = 1 << 0x04 | 1 << 0x05 | 1 << 0x08 | 1 << 0x09 | 1 << 0x10 | 1 << 0x11;
/// CQE opcode of an error completion, and the send opcode (4.7, 5.7).
const CQE_ERROR: u8 = 0x1e;
const OP_SEND: u32 = 0x0a;
/// How long to wait for link after INIT_PORT (5.8), and for a reply.
const LINK_WAIT_MS: u32 = 10_000;
const REPLY_WAIT_MS: u32 = 6_000;
/// Received frames logged one line each, per port.
const LOG_FRAMES: u32 = 32;

fn ms(n: u32) -> Duration {
    Duration::from_millis(u64::from(n))
}

fn wmb() {
    fence(Ordering::SeqCst);
}

/// A run of MTT entries in a context: byte offset bits 39:32 and 31:0 (4.2).
fn set_mtt(m: &Mem, hi: usize, lo: usize, offset: u64) {
    m.set_u8(hi, (offset >> 32) as u8);
    m.set_be32(lo, offset as u32);
}

struct Mac([u8; 6]);

impl core::fmt::Display for Mac {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let m = &self.0;
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5])
    }
}

struct Ip([u8; 4]);

impl core::fmt::Display for Ip {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

/// MTT entries, written straight into the ICM that backs the MTT table (4.2),
/// allocated upward from the first free entry, each run aligned to its
/// rounded-up size as Linux's buddy allocator would.
struct Mtt {
    mem: Mem,
    entry: u64,
    next: u64,
    end: u64,
}

impl Mtt {
    /// Describe `buf` (whole 4 KiB pages) and return the run's byte offset.
    fn map(&mut self, buf: Mem) -> Result<u64, Fail> {
        let n = (buf.len() / PAGE) as u64;
        let first = self.next.next_multiple_of(fw::pow2(n));
        if first + n > self.end {
            return Err(fail!("MTT table full: {n} entries from {first}, table ends at {}", self.end));
        }
        for i in 0..n {
            self.mem.set_be64(((first + i) * self.entry) as usize, (buf.dev + i * PAGE as u64) | 1);
        }
        // Coherent memory: a store fence before the create command suffices.
        wmb();
        self.next = first + n;
        Ok(first * self.entry)
    }
}

/// The event queue (4.6), polled.
struct Eq {
    eqn: u32,
    mem: Mem,
    ci: u32,
}

impl Eq {
    /// Consume and log every event, writing the consumer index at least
    /// every 0x80 entries (4.6).
    fn poll(&mut self, pci: &mut PciIo) {
        let mut n = 0;
        loop {
            let e = self.mem.slice((self.ci % EQ_ENTRIES) as usize * EQE, EQE);
            if u32::from(e.u8(0x1f) >> 7) != (self.ci / EQ_ENTRIES) & 1 {
                break;
            }
            wmb();
            let (kind, sub) = (e.u8(0x01), e.u8(0x03));
            let obj = e.be32(0x04) & 0xff_ffff;
            match kind {
                0x09 => uefi::println!(
                    "  event: port {} {}",
                    e.be32(0x0c) >> 28,
                    match sub {
                        1 => "link down",
                        4 => "link active",
                        _ => "change",
                    }
                ),
                0x04 => uefi::println!(
                    "  event: CQ {obj:#x} error, {}",
                    if e.u8(0x0f) == 1 { "overrun" } else { "access violation" }
                ),
                0x05 | 0x10 | 0x11 => uefi::println!("  event: QP {obj:#x} error, type {kind:#04x}"),
                0x08 => uefi::println!("  event: local catastrophic error"),
                _ => uefi::println!("  event: type {kind:#04x} subtype {sub:#04x}, data {obj:#x}"),
            }
            self.ci = self.ci.wrapping_add(1);
            n += 1;
            if n % 0x80 == 0 {
                self.doorbell(pci);
            }
        }
        if n % 0x80 != 0 {
            self.doorbell(pci);
        }
    }

    /// Consumer index, not re-armed (4.6), in UAR page eqn / 4 (1.4).
    fn doorbell(&self, pci: &mut PciIo) {
        let off = u64::from(self.eqn / 4) * PAGE as u64 + 0x800 + 8 * u64::from(self.eqn % 4);
        let _ = pci.mem_write32(BAR_UAR, off, (self.ci & 0xff_ffff).to_be());
    }
}

/// A completion queue (4.7) and its doorbell record.
struct Cq {
    n: u32,
    mem: Mem,
    db: Mem,
    ci: u32,
}

impl Cq {
    /// The next completion, if software owns it.
    fn peek(&self) -> Option<Mem> {
        let e = self.mem.slice((self.ci % self.n) as usize * CQE, CQE);
        if u32::from(e.u8(0x1f) >> 7) != (self.ci / self.n) & 1 {
            return None;
        }
        wmb();
        Some(e)
    }

    /// Done with it: tell the device through the doorbell record.
    fn pop(&mut self) {
        self.ci = self.ci.wrapping_add(1);
        self.db.set_be32(0, self.ci & 0xff_ffff);
    }
}

/// Objects shared by the ports (4.1, 4.3, 4.6).
struct Shared {
    pd: u32,
    uar: u32,
    lkey: u32,
    eq: Eq,
    mtt: Mtt,
    /// Next free overflow (AMGM) entry in the MCG table (5.4.1).
    next_amgm: u32,
    mcgs: u32,
    b0: bool,
    /// VEP_MC_STEER: MGID_HASH op_mod and the RQP_CALC multicast mode.
    mc_steer: bool,
    /// INIT_HCA enabled counters (capability bit 48).
    counters: bool,
    lb_src_chk: bool,
    port_remap: bool,
    sw_cq_init: bool,
}

/// One port's queues and counters.
struct Port {
    num: u8,
    mac: [u8; 6],
    uar: u32,
    lkey: u32,
    tx_qpn: u32,
    rx_cq: Cq,
    tx_cq: Cq,
    /// The RX doorbell record and the frame buffers.
    rx_db: Mem,
    rx_bufs: Mem,
    rx_prod: u32,
    sq: Mem,
    tx_bufs: Mem,
    tx_prod: u32,
    tx_done: u32,
    stats: Stats,
}

#[derive(Default)]
struct Stats {
    rx: u32,
    broadcast: u32,
    own: u32,
    dropped: u32,
    sent: u32,
    tx_errors: u32,
}

pub fn run(hca: &mut Hca, pci: &mut PciIo, s: &Setup) -> Result<(), Fail> {
    if !s.rev3 {
        uefi::println!("  data path: needs command interface revision 3 (5.5); not started");
        return Ok(());
    }
    let mut sh = shared(hca, pci, s)?;
    for p in &s.ports {
        if !is_ethernet(hca, pci, s, p)? {
            continue;
        }
        let mark = hca.mark();
        let r = check_port(hca, pci, &mut sh, s, p);
        // Take the port down whatever happened (6.4 steps 1–3).
        let down = hca.undo_to(pci, mark);
        sh.eq.poll(pci);
        r?;
        down?;
        uefi::println!("  port {}: closed, QPs reset, CQs returned", p.num);
    }
    Ok(())
}

/// 5.1: Ethernet-only ports, and VPI ports the firmware (or SENSE_PORT)
/// puts on Ethernet.
fn is_ethernet(hca: &mut Hca, pci: &mut PciIo, s: &Setup, p: &PortInfo) -> Result<bool, Fail> {
    let eth = match p.b0 & 3 {
        2 => true,
        3 if s.cap.flag(12) && s.cap.flag(55) && p.b0 & 0x10 != 0 => {
            match hca.hcr.imm(pci, hcr::SENSE_PORT, 0, u32::from(p.num), 0) {
                Ok(v) => {
                    let what = match v {
                        1 => "IB",
                        2 => "Ethernet",
                        _ => "nothing",
                    };
                    uefi::println!("  port {}: SENSE_PORT says {what}", p.num);
                    v == 2
                }
                Err(e) if e.needs_reset() => return hca.cmd(Err(e)),
                // Sensing failed: fall back to the firmware's suggestion.
                Err(_) => p.b0 & 0x08 != 0,
            }
        }
        3 => p.b0 & 0x08 != 0,
        _ => false,
    };
    if !eth {
        uefi::println!("  port {}: not Ethernet (QUERY_PORT byte 0 {:#04x}); skipped", p.num, p.b0);
    }
    Ok(eth)
}

/// 6.1 steps 15–18: object numbers, EQ, special QPs, memory region.
fn shared(hca: &mut Hca, pci: &mut PciIo, s: &Setup) -> Result<Shared, Fail> {
    let (cap, t) = (&s.cap, &s.prof.t);
    if cap.entry[fw::MTT] != 8 {
        return Err(fail!("MTT entry size {} is not 8 bytes (4.2); stopping", cap.entry[fw::MTT]));
    }
    // 4.1
    let pd = cap.rsvd_pds as u32;
    let uar = cap.rsvd_uars.max(128) as u32;
    let eqn = cap.rsvd_eqs;
    if eqn >= t[fw::EQC].count || eqn >= 256 {
        return Err(fail!("EQ number {eqn} does not fit the {}-entry EQ table (4.1); stopping", t[fw::EQC].count));
    }
    let eqn = eqn as u32;
    if u64::from(uar) >= cap.uar_bytes / PAGE as u64 {
        return Err(fail!("UAR page {uar} is outside the UAR area; stopping"));
    }
    if u32::from(cap.log_max_cqes) < fw::log2(u64::from(RX_ENTRIES.max(TX_TXBBS))) + 1
        || u32::from(cap.log_max_qp_wqes) < fw::log2(u64::from(RX_ENTRIES.max(TX_TXBBS)))
    {
        return Err(fail!("queues of {RX_ENTRIES} entries exceed the device's limits; stopping"));
    }
    let mut mtt = Mtt { mem: hca.mtt_mem(), entry: 8, next: s.prof.first_free_mtt, end: t[fw::MTT].count };
    uefi::println!("  objects: PD {pd:#x}, UAR page {uar:#x}, EQ {eqn:#x}, first MTT {}", mtt.next);

    // 4.6: the EQ, every entry hardware-owned for the first pass.
    let mem = hca.lend(pci, "EQ", EQ_ENTRIES as usize * EQE)?;
    for i in 0..EQ_ENTRIES as usize {
        mem.set_u8(i * EQE + 0x1f, 0x80);
    }
    let off = mtt.map(mem)?;
    let m = &hca.hcr.inbox;
    m.zero();
    m.set_be32(0x00, 0x0000_0900);
    m.set_u8(0x0c, fw::log2(u64::from(EQ_ENTRIES)) as u8);
    set_mtt(m, 0x1b, 0x1c, off);
    let r = hca.hcr.with_in(pci, hcr::SW2HW_EQ, 0, eqn);
    hca.cmd(r)?;
    hca.push_undo(hcr::HW2SW_EQ, 1, eqn, 0);
    match hca.hcr.imm(pci, hcr::MAP_EQ, 0, eqn, EVENT_MASK) {
        Ok(_) => hca.push_undo(hcr::MAP_EQ, 0, eqn | 1 << 31, EVENT_MASK),
        Err(e) if e.needs_reset() => return hca.cmd(Err(e)),
        // Linux only warns (4.6); link state is still polled with QUERY_PORT.
        Err(_) => uefi::println!("  MAP_EQ failed: no port events, polling QUERY_PORT only"),
    }
    let eq = Eq { eqn, mem, ci: 0 };

    // 3.12
    let sqpn = s.prof.base_sqpn as u32;
    let r = hca.hcr.imm(pci, hcr::CONF_SPECIAL_QP, 0, sqpn, 0);
    hca.cmd(r)?;
    hca.push_undo(hcr::CONF_SPECIAL_QP, 0, 0, 0);

    // 4.3: one physical region over all memory. MPT index 1 would give the
    // invalid L_Key 0x100, so skip it.
    let mut mpt = cap.rsvd_mpts as u32;
    if mpt == 1 {
        mpt = 2;
    }
    let mpt = mpt & (t[fw::DMPT].count as u32 - 1);
    let m = &hca.hcr.inbox;
    m.zero();
    m.set_be32(0x00, 0xf002_0f00);
    m.set_be32(0x08, mpt);
    m.set_be32(0x0c, pd | 0x0300_0000);
    m.set_be64(0x18, u64::MAX);
    m.set_be32(0x38, 12);
    let r = hca.hcr.with_in(pci, hcr::SW2HW_MPT, 0, mpt);
    hca.cmd(r)?;
    hca.push_undo(hcr::HW2SW_MPT, 1, mpt, 0);
    let lkey = mpt.rotate_left(8);
    uefi::println!("  memory region: MPT {mpt:#x}, L_Key {lkey:#010x}");

    let counters = cap.flag(48);
    Ok(Shared {
        pd,
        uar,
        lkey,
        eq,
        mtt,
        next_amgm: (t[fw::MCG].count / 2) as u32,
        mcgs: t[fw::MCG].count as u32,
        b0: cap.b0(),
        mc_steer: cap.flag(42),
        counters,
        lb_src_chk: counters && cap.flags2 & (1 << 19) != 0,
        port_remap: cap.bmme & (1 << 24) != 0,
        sw_cq_init: cap.flags2 & (1 << 23) != 0,
    })
}

/// Bring the port up, wait for link, try the round trip.
fn check_port(hca: &mut Hca, pci: &mut PciIo, sh: &mut Shared, s: &Setup, p: &PortInfo) -> Result<(), Fail> {
    let mut port = port_up(hca, pci, sh, s, p)?;
    let mut probe = Probe::new(p.mac);
    let Some(waited) = wait_link(hca, pci, sh, &mut port, &mut probe)? else {
        uefi::println!("  port {}: no link after {} s; not tested", p.num, LINK_WAIT_MS / 1000);
        return Ok(());
    };
    uefi::println!("  port {}: link up after {waited} ms", p.num);
    hca.query_port(pci, p.num)?;
    round_trip(pci, sh, &mut port, &mut probe);
    let st = &port.stats;
    uefi::println!(
        "  port {}: sent {}, received {} ({} broadcast, {} own frames looped back), dropped {}, send errors {}",
        p.num, st.sent, st.rx, st.broadcast, st.own, st.dropped, st.tx_errors
    );
    Ok(())
}

/// 6.1 steps 19–27 for one port.
fn port_up(hca: &mut Hca, pci: &mut PciIo, sh: &mut Shared, s: &Setup, p: &PortInfo) -> Result<Port, Fail> {
    let (t, idx) = (&s.prof.t, u32::from(p.num - 1));
    // 4.1: the RX QP 128-aligned with the 127 numbers above it unused (A0),
    // TX QPs after port 1's RX block; two CQs per port.
    let sqpn = s.prof.base_sqpn as u32;
    let rx_qpn = sqpn + 128 * (2 * idx + 1);
    let tx_qpn = sqpn + 256 + idx;
    let rx_cqn = s.cap.rsvd_cqs as u32 + 2 * idx;
    let tx_cqn = rx_cqn + 1;
    if u64::from(rx_qpn.max(tx_qpn)) >= t[fw::QPC].count || u64::from(tx_cqn) >= t[fw::CQC].count {
        return Err(fail!("port {}: QP/CQ numbers outside the profile; stopping", p.num));
    }
    uefi::println!(
        "  port {}: MAC {}, MTU cap {}, RX QP {rx_qpn:#x} CQ {rx_cqn:#x}, TX QP {tx_qpn:#x} CQ {tx_cqn:#x}, {} steering",
        p.num,
        Mac(p.mac),
        p.mtu_cap,
        if sh.b0 { "B0" } else { "A0" }
    );

    // 4.4: the doorbell records: RX CQ, TX CQ, RX QP, TX QP (unused).
    let db = hca.lend(pci, "doorbell records", PAGE)?;
    let rx_cq = cq(hca, pci, sh, rx_cqn, RX_ENTRIES, db.slice(0, 8))?;
    let tx_cq = cq(hca, pci, sh, tx_cqn, TX_TXBBS, db.slice(8, 8))?;

    // 4.8.2: RX QP buffer: one dummy TXBB, owner bit set, then the RQ.
    let rxq = hca.lend(pci, "RX QP", TXBB + RX_ENTRIES as usize * RX_STRIDE)?;
    rxq.set_be32(0, 0x8000_0000);
    let rq = rxq.slice(TXBB, RX_ENTRIES as usize * RX_STRIDE);
    let rx_bufs = hca.lend(pci, "receive buffers", RX_ENTRIES as usize * BUF)?;
    // 5.6: one 2 KiB buffer per descriptor, the whole ring filled.
    for i in 0..RX_ENTRIES as usize {
        let d = i * RX_STRIDE;
        rq.set_be32(d, BUF as u32);
        rq.set_be32(d + 4, sh.lkey);
        rq.set_be64(d + 8, rx_bufs.dev + (i * BUF) as u64);
    }
    let rx_mtt = sh.mtt.map(rxq)?;
    let rx = Qp {
        qpn: rx_qpn,
        rq_size_stride: (fw::log2(u64::from(RX_ENTRIES)) << 3) as u8,
        sq_size_stride: 0x02,
        cqn_send: rx_cqn,
        cqn_recv: rx_cqn,
        db: db.dev + 16,
        mtt: rx_mtt,
        rx: true,
    };
    qp(hca, pci, sh, p.num, &rx)?;
    let rx_db = db.slice(16, 4);
    wmb();
    rx_db.set_be32(0, RX_ENTRIES & 0xffff);

    // 4.8.2, 5.7: TX QP: the SQ, every TXBB stamped not-yet-valid, plus a
    // spare page for the phantom RQ ([HW-CHECK] 10 says it may not be needed).
    let sq_bytes = TX_TXBBS as usize * TXBB;
    let txq = hca.lend(pci, "TX QP", sq_bytes + PAGE)?;
    for i in 0..TX_TXBBS as usize {
        txq.set_be32(i * TXBB, 0xffff_ffff);
    }
    let tx_bufs = hca.lend(pci, "send buffers", TX_TXBBS as usize * BUF)?;
    let tx_mtt = sh.mtt.map(txq)?;
    let tx = Qp {
        qpn: tx_qpn,
        rq_size_stride: 0,
        sq_size_stride: ((fw::log2(u64::from(TX_TXBBS)) << 3) | 2) as u8,
        cqn_send: tx_cqn,
        cqn_recv: tx_cqn,
        db: db.dev + 24,
        mtt: tx_mtt,
        rx: false,
    };
    qp(hca, pci, sh, p.num, &tx)?;

    set_port(hca, pci, sh, p, rx_qpn)?;

    // 5.5
    let r = hca.hcr.imm(pci, hcr::INIT_PORT, 0, u32::from(p.num), 0);
    hca.cmd(r)?;
    hca.push_undo(hcr::CLOSE_PORT, 0, u32::from(p.num), 0);

    // 5.4.1: B0 steering entries after INIT_PORT: our MAC and broadcast.
    if sh.b0 {
        attach(hca, pci, sh, p.num, p.mac, true, rx_qpn)?;
        attach(hca, pci, sh, p.num, [0xff; 6], false, rx_qpn)?;
    }
    // 5.4.3: no port multicast filter; steering decides.
    let r = hca.hcr.imm(pci, hcr::SET_MCAST_FLTR, 1, u32::from(p.num), 0);
    hca.cmd(r)?;

    Ok(Port {
        num: p.num,
        mac: p.mac,
        uar: sh.uar,
        lkey: sh.lkey,
        tx_qpn,
        rx_cq,
        tx_cq,
        rx_db,
        rx_bufs,
        rx_prod: RX_ENTRIES,
        sq: txq.slice(0, sq_bytes),
        tx_bufs,
        tx_prod: 0,
        tx_done: 0,
        stats: Stats::default(),
    })
}

/// 4.7: a CQ of `n` entries, filled with 0xCC and handed to the device.
fn cq(hca: &mut Hca, pci: &mut PciIo, sh: &mut Shared, cqn: u32, n: u32, db: Mem) -> Result<Cq, Fail> {
    let mem = hca.lend(pci, "CQ", n as usize * CQE)?;
    mem.fill(0xcc);
    let off = sh.mtt.map(mem)?;
    let m = &hca.hcr.inbox;
    m.zero();
    m.set_be32(0x0c, (fw::log2(u64::from(n)) << 24) | sh.uar);
    m.set_u8(0x17, sh.eq.eqn as u8);
    set_mtt(m, 0x1b, 0x1c, off);
    m.set_be64(0x38, db.dev);
    let r = hca.hcr.with_in(pci, hcr::SW2HW_CQ, sh.sw_cq_init as u8, cqn);
    hca.cmd(r)?;
    hca.push_undo(hcr::HW2SW_CQ, 1, cqn, 0);
    // Without SW_CQ_INIT the firmware initialises the entries; stamp them
    // hardware-owned again, as mlx4_en does (4.7). No QP uses the CQ yet.
    for i in 0..n as usize {
        mem.set_u8(i * CQE + 0x1f, mem.u8(i * CQE + 0x1f) | 0x80);
    }
    Ok(Cq { n, mem, db, ci: 0 })
}

/// The context values that differ between the two QPs (4.8.4).
struct Qp {
    qpn: u32,
    rq_size_stride: u8,
    sq_size_stride: u8,
    cqn_send: u32,
    cqn_recv: u32,
    db: u64,
    mtt: u64,
    rx: bool,
}

/// RESET → INIT → RTR → RTS with the same context each time (4.8.3).
fn qp(hca: &mut Hca, pci: &mut PciIo, sh: &Shared, port: u8, q: &Qp) -> Result<(), Fail> {
    let steps = [(hcr::RST2INIT_QP, 1u32), (hcr::INIT2RTR_QP, 2), (hcr::RTR2RTS_QP, 3)];
    for (i, (op, state)) in steps.into_iter().enumerate() {
        let m = &hca.hcr.inbox;
        m.zero();
        let c = m.slice(0x08, 0xf8);
        c.set_be32(0x00, (state << 28) | (0x7 << 16));
        c.set_be32(0x04, sh.pd);
        c.set_u8(0x08, 0xff);
        c.set_u8(0x09, q.rq_size_stride);
        c.set_u8(0x0a, q.sq_size_stride);
        c.set_be32(0x0c, sh.uar);
        c.set_be32(0x10, q.qpn);
        // Primary path. 5.10: the RX QP drops its own multicast when the
        // loopback source check is available.
        if q.rx && sh.lb_src_chk {
            c.set_u8(0x18, 0x02);
            c.set_u8(0x19, 0x80);
        }
        c.set_u8(0x1c, if sh.counters { port - 1 } else { 0xff });
        c.set_u8(0x20, 0x01);
        c.set_u8(0x38, 0x83 | ((port - 1) << 6));
        c.set_be32(0x7c, q.cqn_send);
        if i == 1 && !q.rx && sh.port_remap {
            c.set_be32(0x90, 0x08);
        }
        c.set_be32(0x9c, q.cqn_recv);
        c.set_be64(0xa0, q.db);
        c.set_be32(0xc0, 0x4000_0000);
        set_mtt(&c, 0xcb, 0xcc, q.mtt);
        let r = hca.hcr.with_in(pci, op, 0, q.qpn);
        hca.cmd(r)?;
        if i == 0 {
            hca.push_undo(hcr::TO_RST_QP, 2, q.qpn, 0);
        }
    }
    Ok(())
}

/// 5.3 (and 5.4.2 for A0): MAC table, general settings, RQP_CALC.
fn set_port(hca: &mut Hca, pci: &mut PciIo, sh: &Shared, p: &PortInfo, rx_qpn: u32) -> Result<(), Fail> {
    let port = u32::from(p.num);
    let mac = p.mac.iter().fold(0u64, |a, &b| a << 8 | u64::from(b));
    let m = &hca.hcr.inbox;
    m.zero();
    m.set_be64(0, 1 << 63 | mac);
    let r = hca.hcr.with_in(pci, hcr::SET_PORT, 1, 2 << 8 | port);
    hca.cmd(r)?;

    let m = &hca.hcr.inbox;
    m.zero();
    m.set_u8(0x03, 0x07);
    m.set_be16(0x06, PORT_MTU);
    m.set_u8(0x08, 0x80);
    m.set_u8(0x0c, 0x80);
    let r = hca.hcr.with_in(pci, hcr::SET_PORT, 1, port);
    hca.cmd(r)?;

    if !sh.b0 {
        let m = &hca.hcr.inbox;
        m.zero();
        m.set_be32(0x00, rx_qpn);
        m.set_u8(0x05, p.log_macs.min(7));
        m.set_u8(0x0f, 1);
        m.set_be32(0x14, rx_qpn);
        m.set_be32(0x18, (if sh.mc_steer { 1 } else { 2 }) << 30 | rx_qpn);
        let r = hca.hcr.with_in(pci, hcr::SET_PORT, 1, 1 << 8 | port);
        hca.cmd(r)?;
    }
    Ok(())
}

/// 5.4.1: add `qpn` to the MCG entry for this MAC, creating it if needed.
fn attach(hca: &mut Hca, pci: &mut PciIo, sh: &mut Shared, port: u8, mac: [u8; 6], unicast: bool, qpn: u32) -> Result<(), Fail> {
    let mut gid = [0u8; 16];
    gid[5] = port;
    gid[7] = if unicast { 0x02 } else { 0x00 };
    gid[10..].copy_from_slice(&mac);
    let entry = 1usize << fw::LOG_MGM_ENTRY;

    hca.hcr.inbox.zero();
    hca.hcr.inbox.write(0, &gid);
    let r = hca.hcr.in_imm(pci, hcr::MGID_HASH, sh.mc_steer as u8, 0);
    let hash = (hca.cmd(r)? & 0xffff) as u32;

    // Walk the chain from the hash slot: a free slot, our GID, or its end.
    let (mut at, mut last) = (hash, None);
    let found = loop {
        let r = hca.hcr.with_out(pci, hcr::READ_MCG, 0, at);
        hca.cmd(r)?;
        let o = &hca.hcr.outbox;
        let members = o.be32(0x04);
        if members & 0xff_ffff == 0 && at == hash {
            break None;
        }
        let mut g = [0u8; 16];
        o.read(0x10, &mut g);
        if g == gid && members >> 30 == 1 {
            break Some(members & 0xff_ffff);
        }
        let next = o.be32(0x00) >> 6;
        if next == 0 {
            last = Some(at);
            at = sh.next_amgm;
            if at >= sh.mcgs {
                return Err(fail!("MCG table full"));
            }
            sh.next_amgm += 1;
            break None;
        }
        at = next;
    };

    let m = &hca.hcr.inbox;
    let count = match found {
        Some(n) => {
            m.zero();
            for i in (0..entry).step_by(4) {
                m.set_be32(i, hca.hcr.outbox.be32(i));
            }
            if (0..n as usize).any(|i| m.be32(0x20 + 4 * i) & 0xff_ffff == qpn) {
                return Ok(());
            }
            n
        }
        None => {
            m.zero();
            m.write(0x10, &gid);
            0
        }
    };
    if 0x20 + 4 * (count as usize + 1) > entry {
        return Err(fail!("MCG entry {at:#x} is full"));
    }
    m.set_be32(0x20 + 4 * count as usize, qpn);
    m.set_be32(0x04, (count + 1) | 1 << 30);
    let r = hca.hcr.with_in(pci, hcr::WRITE_MCG, 0, at);
    hca.cmd(r)?;

    // A new overflow entry: link it from the end of the chain.
    if let Some(prev) = last {
        let r = hca.hcr.with_out(pci, hcr::READ_MCG, 0, prev);
        hca.cmd(r)?;
        let m = &hca.hcr.inbox;
        for i in (0..entry).step_by(4) {
            m.set_be32(i, hca.hcr.outbox.be32(i));
        }
        m.set_be32(0x00, at << 6);
        let r = hca.hcr.with_in(pci, hcr::WRITE_MCG, 0, prev);
        hca.cmd(r)?;
    }
    uefi::println!(
        "  steering: {} {} -> QP {qpn:#x} (MCG entry {at:#x}, hash {hash:#x})",
        if unicast { "unicast" } else { "multicast" },
        Mac(mac)
    );
    Ok(())
}

/// Poll QUERY_PORT (quietly) until the link is up (5.8); returns the wait.
fn wait_link(hca: &mut Hca, pci: &mut PciIo, sh: &mut Shared, port: &mut Port, probe: &mut Probe) -> Result<Option<u32>, Fail> {
    hca.hcr.quiet = true;
    let mut waited = 0;
    let r = loop {
        match hca.query_port(pci, port.num) {
            Ok(i) if i.link_up => break Ok(Some(waited)),
            Ok(_) if waited >= LINK_WAIT_MS => break Ok(None),
            Ok(_) => {}
            Err(f) => break Err(f),
        }
        port.service(pci, sh, probe);
        boot::stall(ms(100));
        waited += 100;
    };
    hca.hcr.quiet = false;
    r
}

/// Broadcast a DHCPDISCOVER, then listen; once an address has been seen in
/// someone's ARP request, also broadcast an ARP probe for it.
fn round_trip(pci: &mut PciIo, sh: &mut Shared, port: &mut Port, probe: &mut Probe) {
    uefi::println!("  port {}: broadcasting DHCPDISCOVER (xid {:#010x})", port.num, probe.xid);
    port.send(pci, &probe.discover());
    let mut arp_sent = None;
    let mut waited = 0;
    while probe.reply.is_none() && waited < REPLY_WAIT_MS {
        port.service(pci, sh, probe);
        if let Some(ip) = probe.learned {
            if arp_sent != Some(ip) && waited >= 1000 {
                uefi::println!("  port {}: broadcasting an ARP probe for {}", port.num, Ip(ip));
                port.send(pci, &probe.arp(ip));
                arp_sent = Some(ip);
            }
        }
        boot::stall(ms(10));
        waited += 10;
    }
    port.service(pci, sh, probe);
    match probe.reply.take() {
        Some(what) => uefi::println!("  port {}: broadcast round trip ok: {what}", port.num),
        None => uefi::println!("  port {}: no reply to the broadcast within {} s", port.num, REPLY_WAIT_MS / 1000),
    }
}

impl Port {
    /// Poll the EQ, reclaim sent frames, take every received frame.
    fn service(&mut self, pci: &mut PciIo, sh: &mut Shared, probe: &mut Probe) {
        sh.eq.poll(pci);
        self.reap_tx();
        let mut frame = [0u8; BUF];
        while let Some(len) = self.recv(&mut frame) {
            if let Some(len) = len {
                self.note(&frame[..len], probe);
            }
        }
    }

    /// One received frame into `out` (5.6): `None` when there is none,
    /// `Some(None)` for one that was dropped.
    fn recv(&mut self, out: &mut [u8; BUF]) -> Option<Option<usize>> {
        let e = self.rx_cq.peek()?;
        let slot = (u32::from(e.be16(0x18)) % RX_ENTRIES) as usize;
        let r = if e.u8(0x1f) & 0x1f == CQE_ERROR {
            uefi::println!(
                "  port {} rx: error completion, syndrome {:#04x} (vendor {:#04x})",
                self.num,
                e.u8(0x1b),
                e.u8(0x1a)
            );
            None
        } else if e.u8(0x13) & 0x10 != 0 {
            None
        } else {
            let len = (e.be32(0x14) as usize).min(BUF);
            self.rx_bufs.read(slot * BUF, &mut out[..len]);
            Some(len)
        };
        if r.is_none() {
            self.stats.dropped += 1;
        }
        self.rx_cq.pop();
        // The slot's descriptor still names its buffer: reposting it is
        // just advancing the producer counter.
        self.rx_prod = self.rx_prod.wrapping_add(1);
        wmb();
        self.rx_db.set_be32(0, self.rx_prod & 0xffff);
        Some(r)
    }

    /// Log a received frame and let the probe look at it.
    fn note(&mut self, f: &[u8], probe: &mut Probe) {
        if f.len() < 14 {
            return;
        }
        let st = &mut self.stats;
        st.rx += 1;
        let dst: [u8; 6] = f[0..6].try_into().unwrap();
        let src: [u8; 6] = f[6..12].try_into().unwrap();
        let own = src == self.mac;
        if dst == [0xff; 6] {
            st.broadcast += 1;
        }
        if own {
            st.own += 1;
        }
        if st.rx <= LOG_FRAMES {
            uefi::println!(
                "  port {} rx: {} bytes {} <- {} type {:04x}{}",
                self.num,
                f.len(),
                Mac(dst),
                Mac(src),
                u16::from_be_bytes([f[12], f[13]]),
                if own { " (own frame, looped back)" } else { "" }
            );
        }
        if !own {
            probe.see(f);
        }
    }

    /// Reclaim completed sends and stamp their TXBBs (5.7). Every WQE is one
    /// TXBB and asks for a completion, so completions come in ring order.
    fn reap_tx(&mut self) {
        while let Some(e) = self.tx_cq.peek() {
            if e.u8(0x1f) & 0x1f == CQE_ERROR {
                self.stats.tx_errors += 1;
                uefi::println!(
                    "  port {} tx: error completion, syndrome {:#04x} (vendor {:#04x}); the TX QP is now in error",
                    self.num,
                    e.u8(0x1b),
                    e.u8(0x1a)
                );
            }
            let owner = (self.tx_done / TX_TXBBS) & 1;
            self.sq.set_be32((self.tx_done % TX_TXBBS) as usize * TXBB, 0x7fff_ffff | owner << 31);
            self.tx_done = self.tx_done.wrapping_add(1);
            self.tx_cq.pop();
        }
    }

    /// Queue one frame (5.7): copy it into the slot's buffer, build a
    /// control + data segment WQE, hand it over, ring the doorbell.
    fn send(&mut self, pci: &mut PciIo, frame: &[u8]) -> bool {
        self.reap_tx();
        if self.tx_prod.wrapping_sub(self.tx_done) >= TX_TXBBS - HEADROOM - 1 || frame.len() > BUF {
            uefi::println!("  port {} tx: ring full, frame not sent", self.num);
            return false;
        }
        let slot = (self.tx_prod % TX_TXBBS) as usize;
        let len = frame.len().max(MIN_FRAME);
        let b = self.tx_bufs.slice(slot * BUF, BUF);
        b.write(0, frame);
        for i in frame.len()..len {
            b.set_u8(i, 0);
        }
        let w = self.sq.slice(slot * TXBB, TXBB);
        // Data segment: address and key, then the byte count.
        w.set_be32(0x14, self.lkey);
        w.set_be64(0x18, b.dev);
        wmb();
        w.set_be32(0x10, len as u32);
        // Control segment: no VLAN, 2 × 16 bytes, CQ update + solicited.
        w.set_be16(0x04, 0);
        w.set_u8(0x06, 0);
        w.set_u8(0x07, 2);
        w.set_be32(0x08, 0x0000_000e);
        w.set_be32(0x0c, 0);
        wmb();
        let owner = if self.tx_prod & TX_TXBBS != 0 { 1 << 31 } else { 0 };
        w.set_be32(0x00, owner | OP_SEND);
        self.tx_prod = self.tx_prod.wrapping_add(1);
        wmb();
        // 4.8.6: QPN << 8, big-endian, at UAR + 0x14.
        let off = u64::from(self.uar) * PAGE as u64 + 0x14;
        if let Err(e) = pci.mem_write32(BAR_UAR, off, (self.tx_qpn << 8).to_be()) {
            uefi::println!("  port {} tx: doorbell write failed ({:?})", self.num, e.status());
        }
        self.stats.sent += 1;
        true
    }
}

/// The round-trip check: what was sent and what counts as its answer.
struct Probe {
    mac: [u8; 6],
    xid: u32,
    /// An IPv4 address seen as the sender of someone's ARP request.
    learned: Option<[u8; 4]>,
    reply: Option<alloc::string::String>,
}

impl Probe {
    fn new(mac: [u8; 6]) -> Self {
        let xid = u32::from_be_bytes([mac[2], mac[3], mac[4], mac[5]]) ^ 0x6d6c_7834;
        Probe { mac, xid, learned: None, reply: None }
    }

    fn ether(&self, kind: u16, payload: usize) -> Vec<u8> {
        let mut f = vec![0u8; (14 + payload).max(MIN_FRAME)];
        f[0..6].fill(0xff);
        f[6..12].copy_from_slice(&self.mac);
        f[12..14].copy_from_slice(&kind.to_be_bytes());
        f
    }

    /// DHCPDISCOVER from 0.0.0.0:68 to 255.255.255.255:67 with the broadcast
    /// flag, so the offer comes back as a broadcast too (RFC 2131).
    fn discover(&self) -> Vec<u8> {
        const BOOTP: usize = 300;
        let mut f = self.ether(0x0800, 20 + 8 + BOOTP);
        let ip = &mut f[14..34];
        ip[0] = 0x45;
        ip[2..4].copy_from_slice(&((20 + 8 + BOOTP) as u16).to_be_bytes());
        ip[8] = 64;
        ip[9] = 17;
        ip[16..20].fill(0xff);
        let sum = checksum(ip);
        ip[10..12].copy_from_slice(&sum.to_be_bytes());
        let udp = &mut f[34..42];
        udp[0..2].copy_from_slice(&68u16.to_be_bytes());
        udp[2..4].copy_from_slice(&67u16.to_be_bytes());
        udp[4..6].copy_from_slice(&((8 + BOOTP) as u16).to_be_bytes());
        let b = &mut f[42..42 + BOOTP];
        b[0..4].copy_from_slice(&[1, 1, 6, 0]);
        b[4..8].copy_from_slice(&self.xid.to_be_bytes());
        b[10] = 0x80;
        b[28..34].copy_from_slice(&self.mac);
        b[236..240].copy_from_slice(&[99, 130, 83, 99]);
        // Message type DISCOVER; ask for subnet mask, router, DNS; end.
        b[240..249].copy_from_slice(&[53, 1, 1, 55, 3, 1, 3, 6, 255]);
        f
    }

    /// ARP probe (RFC 5227): who has `ip`, from sender address 0.0.0.0.
    fn arp(&self, ip: [u8; 4]) -> Vec<u8> {
        let mut f = self.ether(0x0806, 28);
        let a = &mut f[14..42];
        a[0..8].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
        a[8..14].copy_from_slice(&self.mac);
        a[24..28].copy_from_slice(&ip);
        f
    }

    /// Is this frame the answer? Learn an address from ARP requests.
    fn see(&mut self, f: &[u8]) {
        use core::fmt::Write;
        let src = Mac(f[6..12].try_into().unwrap());
        match u16::from_be_bytes([f[12], f[13]]) {
            0x0806 if f.len() >= 42 => {
                let a = &f[14..42];
                let spa: [u8; 4] = a[14..18].try_into().unwrap();
                match a[7] {
                    1 if spa != [0; 4] && self.learned.is_none() => self.learned = Some(spa),
                    2 if a[18..24] == self.mac => {
                        let mut s = alloc::string::String::new();
                        let _ = write!(s, "ARP reply: {} is at {src}", Ip(spa));
                        self.reply = Some(s);
                    }
                    _ => {}
                }
            }
            0x0800 if f.len() >= 34 && f[23] == 17 => {
                let ihl = usize::from(f[14] & 0xf) * 4;
                let (udp, bootp) = (14 + ihl, 14 + ihl + 8);
                if f.len() < bootp + 240 || f[udp + 2..udp + 4] != 68u16.to_be_bytes() {
                    return;
                }
                let b = &f[bootp..];
                if b[0] != 2 || b[4..8] != self.xid.to_be_bytes() {
                    return;
                }
                let yiaddr = Ip(b[16..20].try_into().unwrap());
                let (mut kind, mut server) = (0u8, None);
                let mut i = 240;
                while i + 1 < b.len() && b[i] != 255 {
                    if b[i] == 0 {
                        i += 1;
                        continue;
                    }
                    let n = usize::from(b[i + 1]);
                    match b[i] {
                        53 if n >= 1 && i + 2 < b.len() => kind = b[i + 2],
                        54 if n == 4 && i + 6 <= b.len() => server = Some(Ip(b[i + 2..i + 6].try_into().unwrap())),
                        _ => {}
                    }
                    i += 2 + n;
                }
                let name = if kind == 2 { "DHCPOFFER" } else { "DHCP reply" };
                let mut s = alloc::string::String::new();
                let _ = write!(s, "{name} of {yiaddr} from {src}");
                if let Some(ip) = server {
                    let _ = write!(s, " (server {ip})");
                }
                self.reply = Some(s);
            }
            _ => {}
        }
    }
}

/// The IPv4 header checksum (RFC 791).
fn checksum(h: &[u8]) -> u16 {
    let mut sum: u32 = h.chunks(2).map(|c| u32::from(u16::from_be_bytes([c[0], c[1]]))).sum();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}
