# Changelog

## [Unreleased]

### 2026-09-27
- **feat:** Driver scaffold (#1): linked as an EFI boot-service driver (`build.rs`), installs `EFI_DRIVER_BINDING_PROTOCOL`, and `Supported` matches ConnectX-3 15b3:1003 / ConnectX-3 Pro 15b3:1007 through a minimal `EFI_PCI_IO_PROTOCOL` binding (`src/pci.rs`). A NIC that already has an SNP, or whose PCI I/O another driver holds, is left alone. `Start` logs the bind and returns `UNSUPPORTED` until bring-up exists. `scripts/pe-subsystem.sh` checks that the image is a boot-service driver
- **chore:** Project created: a Rust `no_std` UEFI SNP driver for ConnectX-3, replacing iPXE's `ipxe-hermon.efi` on the stormbootx media (stormbootx#26, #27). Scaffold only
