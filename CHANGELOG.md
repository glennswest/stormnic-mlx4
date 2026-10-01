# Changelog

## [Unreleased]

<!-- New unreleased changes go here -->

### 2026-10-01
- **docs:** #15 verified on server3 and closed; v0.2.2 pin requested in stormbootx#66 (work plan).
- **docs:** #15 released as v0.2.1, waiting on the rustnic pin (stormbootx#64) and a server3 boot (work plan).

### 2026-09-29
- **docs:** #3 pass restated for v0.2.0 (`port N SNP: initialized, media present`, `tcp4 : available`, traffic past it) (work plan).
- **docs:** #2 pass restated for v0.2.0 (`INIT_HCA (0x0): ok`, `firmware bring-up complete`, `port N SNP: initialized`); stale stormbootx#45 reference fixed (work plan).
- **docs:** #1 recheck: server1 has still not booted the rustnic media; pass accepts 0.1.0 or 0.2.0, current golden named (work plan).
- **docs:** #4 waits on stormbootx#50 (pin v0.2.0 on the rustnic media) for the server1 `tcp4 : available` check

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
