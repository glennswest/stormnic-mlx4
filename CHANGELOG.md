# Changelog

## [Unreleased]

### 2026-09-28
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
