# Changelog

## [Unreleased]

<!-- New unreleased changes go here -->

### 2026-10-06
- **docs:** #5 done. `ipxe-hermon.efi` is retired: stormbootx#91 removed iPXE from every medium. server3's rustnic boots, with no iPXE, claimed `boothost/server3` and attached their image through the ConnectX-3 alone. README and CLAUDE.md drop the `IPXE_DRIVERS="intelx hermon"` opt-in, and name stormbootx's `STORMNIC_DRIVERS` (not `STORMNIC_ON_MEDIA`) and the driverless fw medium (#5).
- **docs:** licensed MIT (LICENSE added); NOTICE is now an acknowledgement of where the hardware facts were learned (an original Rust rewrite, no code copied) — owner, repo made public
- **docs:** Spec §7.1 records HW-checks 15 and 16 from all 34 rustnic SOL captures (server1, 3, 4, 7, 8; firmware 2.30.8000). 16: 256 reserved MPTs give MPT 0x100 and L_Key 0x00010000, so there is no collision with 0x100. 15: not observable yet. Every attached boot reaches `STARTING KERNEL`, but the started image's kernel console is ttyS0 and the X9 SOL is ttyS1 (stormcos#220), so the OS-side `mlx4_core` probe is not captured (#12). README status updated.

### 2026-10-06
- **docs:** v0.2.5 verified in sc-build (both images, host tests 9 + 8 + 3); stormbootx#104 now asks for v0.2.5; work plan and README pin text (#16, #17).

### 2026-10-06
- **feat:** Quiet console by default (#16), the same switch as stormnic-ixgbe#22: one line per port on a good Start (`stormnic-mlx4 X.Y.Z: LOC 15b3:DDDD NAME port N: MAC …, link up SPEED|no link, SNP installed`) plus every warning and error. The command trace, ICM sizes, QUERY_DEV_CAP bytes, SNP calls, per-frame lines and the #17 diagnostics on a linked port are printed only when verbose: the EFI variable `StormnicVerbose` (vendor GUID ce1479a2-eab9-4176-b0ad-c909ea5b8e0b, first byte non-zero, read once at the entry point) or the `verbose` build feature. When quiet, the last 16 trace lines are kept and replayed ahead of any failure. A port without link still prints its QUERY_PORT and link-diagnostic lines. Diagnostic commands (MAD_IFC, ACCESS_REG) report their own status instead of a failure replay. New `src/console.rs`, `src/trace.rs`, host test `test/trace.rs`.
- **docs:** README "Console output" (what is always printed, the verbose switch, the replay); CLAUDE.md logging rule and interfaces (#16).
- **chore:** v0.2.5.

### 2026-10-06
- **docs:** v0.2.4 verified in sc-build (subsystem 11, host tests 9 + 8); pin requested in stormbootx#104; README and work plan record the v0.2.3 pin and the #17 hardware check left (#17).

### 2026-10-06
- **feat:** DAC link diagnostics (#17, spec 5.11–5.13). After the link wait and at every link-down, each port prints its module EEPROM read through MAD_IFC attribute 0xFF60 (identifier with HCR and MAD status, cable-info errors by name, raw bytes, SFF labels: passive/active cable, length, connector, compliance, vendor, part, serial) and a read-only ACCESS_REG PTYS query (supported/advertised/operating/partner link modes; on firmware without QUERY_DEV_CAP 0x7a bit 5 it is spec 7 item 18's one-off diagnostic, not repeated once refused). A `speed control:` line says whether the card offers a forced speed; the driver never writes PTYS. QUERY_DEV_CAP logs ETH_PROT_CTRL and ETH_BACKPL_AN_REP; the link line shows autonegotiation enabled and complete. New `src/diag.rs`, `src/module.rs`, host test `test/module.rs`.
- **test:** `test/module.rs` allows the driver-only constants (no dead-code warnings).
- **chore:** v0.2.4.

### 2026-10-06
- **docs:** Spec addition by an independent agent from the BSD option of the Linux/FreeBSD mlx4 sources (new pinned commits in §0.2 and NOTICE): §5.11 module EEPROM read (MAD_IFC op_mod 3, attribute 0xFF60, 48-byte chunks, cable-info error codes, SFF byte meanings), §5.12 ACCESS_REG and the PTYS register (layout, link-mode bit table, gated on QUERY_DEV_CAP 0x7a bit 5), §5.13 forcing speed/autoneg (no SET_PORT path; PTYS admin write only, a real autoneg-off bit only for 1G), §7 HW-checks 17–19, rows in §2.9, §3.4, §3.5 and Appendices A, C, F. The blades' firmware 2.30.8000 reports 0x7a = 0x00, so PTYS (advertised masks, forced speed) is not offered there (#20).

### 2026-10-06
- **docs:** README status and pin updated: RX and a lease are on record on server3 and server8, §7.1 holds the HW-check results, and stormbootx#66 now asks for v0.2.3. Work plan: #18 done, #21 open (#18).

### 2026-10-06
- **feat:** The SNP logs the first frame from the port's own MAC that the adapter loops back, before it drops it (`port N rx: own frame looped back …`). This lets the console answer spec §7 item 14 (#21).
- **docs:** Spec §7.1 records HW-checks 1–14 on the X9 blades' ConnectX-3 (firmware 2.30.8000), from the server3 and server8 rustnic boots at v0.2.1. Those boots also show RX, a DHCP lease, and a boothost claim and NVMe/TCP attach through this driver. Item 14 (own-frame loopback) is not visible on the console and moves to #21 (#18).

### 2026-10-04
- **docs:** Documentation refreshed from the code. README: stormbootx runs smoltcp over the SNP (not the firmware's TCP4) and opens it `EXCLUSIVE`; shipping pin is `cf37f8b` (v0.2.1, v0.2.2 in stormbootx#66) and the hardware check is `tcp4 : smoltcp over SNP` plus `port N rx:` (#19); status says #1–#4 were checked on server3/server1 with RX and §7 still open (#18, #12); console sample shows 0.2.2. CLAUDE.md: shipping and Test sections updated (blade boots are the master's job, not `needs-owner`; host tests), work plan collapsed to done/open with the remaining hardware work on #18, #12, #17/#20 (#14). No code change.

### 2026-10-01
- **docs:** #15 verified on server3 and closed; v0.2.2 pin requested in stormbootx#66 (work plan).
- **docs:** #15 released as v0.2.1, waiting on the rustnic pin (stormbootx#64) and a server3 boot (work plan).

### 2026-09-29
- **docs:** #3 pass restated for v0.2.0 (`port N SNP: initialized, media present`, `tcp4 : available`, traffic past it) (work plan).
- **docs:** #2 pass restated for v0.2.0 (`INIT_HCA (0x0): ok`, `firmware bring-up complete`, `port N SNP: initialized`); stale stormbootx#45 reference fixed (work plan).
- **docs:** #1 recheck: server1 has still not booted the rustnic media; pass accepts 0.1.0 or 0.2.0, current golden named (work plan).
- **docs:** #4 waits on stormbootx#50 (pin v0.2.0 on the rustnic media) for the server1 `tcp4 : available` check

## [v0.2.5] — 2026-10-06

### Added
- Quiet console by default (#16): one line per port plus warnings and errors. The bring-up trace is printed only with the EFI variable `StormnicVerbose` (GUID ce1479a2-eab9-4176-b0ad-c909ea5b8e0b, shared with stormnic-ixgbe) or `--features verbose`; otherwise the last 16 trace lines are replayed ahead of a failure. Host test `test/trace.rs`.

## [v0.2.4] — 2026-10-06

### Added
- DAC link diagnostics (#17, spec 5.11–5.13): module EEPROM through MAD_IFC 0xFF60 (statuses, identifier, raw bytes, SFF labels), a read-only ACCESS_REG PTYS query (link-mode masks; spec 7 item 18 on firmware without ETH_PROT_CTRL), and a `speed control:` line. Printed after the link wait and at every link-down. No speed is forced. Host test `test/module.rs`.

## [v0.2.3] — 2026-10-06

### Added
- The SNP logs the first frame from the port's own MAC that the adapter loops back (`port N rx: own frame looped back …`), so the console answers spec §7 item 14 (#21).

### Documentation
- Spec §7.1: HW-checks 1–14 recorded from the server3 and server8 rustnic boots, which also show RX and a DHCP lease through this driver (#18).

## [v0.2.2] — 2026-10-01

### Added
- Link diagnostics (#15): at the 5 s link wait and after every link change the driver prints QUERY_PORT's speed code (named), autonegotiation, and the module's transceiver type, vendor OUI, wavelength and code (spec 3.5), so a port without link can be matched to the switch side.

## [v0.2.1] — 2026-10-01

### Fixed
- UAR BAR found by asking PCI I/O instead of hard-coding `BarIndex` 2 (#15): AMI Aptio 4 (X9) numbers BARs, not BAR registers, so index 2 was refused and every doorbell write failed UNSUPPORTED. `src/bars.rs` takes the candidate (register index, BAR count) whose `GetBarAttributes` base is the config-space address, then scans 0–5; the EQ and TX doorbells, the UAR size check and the catastrophic-buffer BAR use it.
- VPI ports are always driven as Ethernet (owner, #15); the firmware's suggestion and SENSE_PORT are only logged, and every port prints the type used and why.

### Added
- `test/bars.rs` (EDK2 and AMI BAR numbering, simulated `GetBarAttributes`), run by `scripts/test-host.sh`.

## [v0.2.0] — 2026-09-29

### Added
- `EFI_SIMPLE_NETWORK_PROTOCOL` (#4). `Start` now keeps the device: firmware and every Ethernet port brought up (spec 6.1), up to 5 s of link wait, then one child handle per Ethernet port with a MAC device path and an SNP (`src/snp.rs`): Start/Stop/Initialize/Shutdown/Reset, Transmit through the bounce buffers with the caller's buffer returned by GetStatus after its completion, Receive (BUFFER_TOO_SMALL leaves the frame queued), unicast/broadcast/multicast receive filters (B0 multicast entries attached on demand, software filter, own looped-back frames dropped), MCastIpToMac, WaitForPacket, MediaPresent from port-change events or QUERY_PORT. The driver binding is now our own so `Stop` can take child handles; `Stop` tears down per spec 6.4 and frees; an ExitBootServices event runs the 6.4 command teardown silently without freeing, releases ownership and clears bus master. The #3 broadcast round-trip self-test is gone from `Start` (cef8dc5 remains the pin that runs it)
- Ethernet data path (#3), written from `docs/spec/connectx3.md` sections 4–6 (`src/eth.rs`): EQ with MAP_EQ, CONF_SPECIAL_QP, one physical memory region, and per Ethernet port receive/send CQs, raw-Ethernet receive and send QPs (RESET→INIT→RTR→RTS), MTT entries written straight into ICM, SET_PORT MAC table/general (+RQP_CALC for A0), INIT_PORT, B0 MCG steering for the port MAC and broadcast, SET_MCAST_FLTR off, polling send/receive and EQ. `Start` waits for link, broadcasts a DHCPDISCOVER (and an ARP probe for an address seen on the wire), logs the frames and whether a reply came back, then closes the port. Every object pushes its undo command, so teardown follows spec 6.4's order before CLOSE_HCA, and queue/frame memory is freed only after UNMAP_FA or a reset. `src/dma.rs` gains a copyable `Mem` view; `src/hcr.rs` the data-path opcodes and a quiet mode for link polling; the profile reserves CQs for two ports. Still returns `UNSUPPORTED` until the SNP (#4)
- Firmware command interface (#2), written from `docs/spec/connectx3.md`: HCR protocol in polling mode with the toggle taken from the T bit after reset (`src/hcr.rs`); ownership semaphore, device reset with config-space restore, QUERY_FW, MAP_FA/RUN_FW, MOD_STAT_CFG, QUERY_DEV_CAP, QUERY_PORT, a small ICM profile laid out largest-first, SET_ICM_SIZE, MAP_ICM_AUX, MAP_ICM, INIT_HCA, QUERY_FUNC, QUERY_ADAPTER, and the teardown CLOSE_HCA → UNMAP_ICM → UNMAP_ICM_AUX → UNMAP_FA with a device-reset fallback (`src/fw.rs`); DMA common buffers and the page-list split (`src/dma.rs`); PCI I/O memory, DMA, attribute and BAR-size calls (`src/pci.rs`). `Start` runs it, logs every command and the spec's hardware-checklist values, tears it down and still returns `UNSUPPORTED` until the data path (#3) exists

### Fixed
- Build failure #11 (implicit autoref through the NIC pointer)

### Changed
- Commit `Cargo.lock` (uefi 0.39.0, uefi-raw 0.15.1; generated on dev) and build with `--locked`, so stormbootx's pinned-commit nic-drivers build is reproducible (#9). Verified at b89d437 with sc-build: locked release UEFI build, PE subsystem 11, lock unchanged. No runtime change or version bump

### Documentation
- #1: rustnic media with mlx4@cef8dc5 exists (stormbootx#34); server1 has not booted it yet, so hardware acceptance waits on the master. Updated the expected `Start` line. No code change
- The driver is written from `docs/spec/connectx3.md` only (owner decision on #2); README and CLAUDE.md rules updated
- `docs/spec/connectx3.md`: ConnectX-3 / ConnectX-3 Pro programming specification for a polling UEFI Ethernet driver (PCI/BARs and reset, HCR command protocol, firmware bring-up and ICM profile, MTT/MPT/EQ/CQ/QP contexts, steering, send/receive WQEs and CQEs, walkthrough, hardware checklist, constants), written by an independent agent from the BSD option of the Linux/FreeBSD mlx4 sources (#10). `NOTICE` carries their BSD notice. No code change
- #2 blocked on an owner decision: the ConnectX-3 PRM is not public (support-contract only), so the documentation source for the firmware command interface must be chosen (get the PRM, use the BSD-licensed mlx4 sources with notice, or park). No code change

## Before v0.2.0 (v0.1.0, never tagged)

### 2026-09-29
- See v0.2.0 above.

### 2026-09-28
- **chore:** Verified #1 at a4c8194 with sc-build (release UEFI build, PE subsystem 11, exit 0); moved hardware acceptance behind stormbootx#34. No runtime change or version bump
- **docs:** Track missing mlx4 media integration in stormbootx#34 after #29 shipped only ixgbe; update shipping references
- **docs:** Recheck #1 acceptance: stormbootx#29 closed with ixgbe integration only; plan a specific mlx4 media dependency and fresh remote build verification
- **chore:** Reverified #1 at f077b04 using sc-build: release UEFI build passes and PE subsystem is 11; queued hardware acceptance after stormbootx#29. No runtime changes or version bump
- **docs:** Align shipping instructions with the owner-approved pinned stormbootx nic-drivers golden and clarify #1 hardware acceptance
- **docs:** Record the #1 verification follow-up and the approved media dependency; hardware acceptance remains required before closure
- **docs:** README and CLAUDE.md refreshed from the code: interfaces (driver binding v1 installed, PCI I/O consumed, no configuration/ports/APIs), how it ships (a single `.efi` laid on stormbootx media with `--drivers`, no golden), stormbootx's load order, hermon now opt-in because it hangs server1, every console outcome, and per-job build drives (no shared `CARGO_TARGET_DIR`). Unlogged error paths filed as #7
- **chore:** Driver-binding scaffold (#1) verified to build on dev as a boot-service driver (PE subsystem 11); hardware check pending

### 2026-09-27
- **docs:** The documented build runs `scripts/pe-subsystem.sh` on `${CARGO_TARGET_DIR:-target}`, since dev builds into a shared target dir (#6)
- **feat:** Driver scaffold (#1): linked as an EFI boot-service driver (`build.rs`), installs `EFI_DRIVER_BINDING_PROTOCOL`, and `Supported` matches ConnectX-3 15b3:1003 / ConnectX-3 Pro 15b3:1007 through a minimal `EFI_PCI_IO_PROTOCOL` binding (`src/pci.rs`). A NIC that already has an SNP, or whose PCI I/O another driver holds, is left alone. `Start` logs the bind and returns `UNSUPPORTED` until bring-up exists. `scripts/pe-subsystem.sh` checks that the image is a boot-service driver
- **chore:** Project created: a Rust `no_std` UEFI SNP driver for ConnectX-3, replacing iPXE's `ipxe-hermon.efi` on the stormbootx media (stormbootx#26, #27). Scaffold only
