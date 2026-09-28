# stormnic-mlx4

A UEFI driver, in Rust (`no_std`), that gives firmware an
`EFI_SIMPLE_NETWORK_PROTOCOL` for **Mellanox ConnectX-3 / ConnectX-3 Pro Ethernet (the Linux `mlx4` family)**.

## Why it exists

stormbootx boots a machine over NVMe/TCP using the firmware's own TCP/IP
stack (`EFI_TCP4`). That stack needs a NIC driver underneath it. Some
platforms have the stack but no UEFI driver for their NIC: the Supermicro X9
blades (server1–8) have only legacy option ROMs for their Intel 10G and
ConnectX-3 ports, so no network handle exists and `EFI_TCP4` never appears
(stormbootx#26).

stormbootx loads every `*.efi` in `\stormboot\drivers` on its boot media
before it looks for TCP4. This driver is one of them. The firmware's MNP, IP4
and TCP4 drivers bind on top of the SNP it installs. There is no PXE, no DHCP
boot and no network code of its own above the link layer.

The interim driver is iPXE's `ipxe-hermon.efi` (GPL-2 C, built from pinned
source). This crate replaces it (stormbootx#27).

## Hardware

- PCI IDs: 15b3:1003 (ConnectX-3), 15b3:1007 (ConnectX-3 Pro); the X9 blades carry 15b3:1003
- References: Mellanox ConnectX-3 Programmer's Reference Manual (PRM); the firmware command interface (HCR, mailboxes, EQ/CQ/QP) is the bulk of the work

**Written from the vendor documentation, not translated from iPXE or Linux.**
Reading other drivers for behaviour is fine; copying their code or structure
would make this a GPL derivative, and it is MIT.

## Build

`x86_64-unknown-uefi`, built on dev with `sc-build` after pushing:

```bash
sc-build 'cargo build --release --target x86_64-unknown-uefi && scripts/pe-subsystem.sh "${CARGO_TARGET_DIR:-target}"/x86_64-unknown-uefi/release/stormnic-mlx4.efi'
```

## Status

Driver binding only (#1). The image is an EFI boot-service driver
(`build.rs` sets the PE subsystem; `scripts/pe-subsystem.sh IMAGE` checks it is
11). Its entry point installs `EFI_DRIVER_BINDING_PROTOCOL` and returns.

- `Supported` accepts a PCI function with vendor/device 15b3:1003 or 15b3:1007,
  unless it already has an SNP or another driver holds its PCI I/O `BY_DRIVER`:
  a platform's own driver always wins.
- `Start` logs the bind, then releases the NIC and returns `UNSUPPORTED`: there
  is no firmware bring-up yet, and holding the device without producing an SNP
  would only keep another driver (such as `ipxe-hermon.efi`) off it.

Every decision about a ConnectX-3 is printed to the console, e.g.

```
stormnic-mlx4 0.1.0: driver binding installed (15b3:1003 ConnectX-3, 15b3:1007 ConnectX-3 Pro)
stormnic-mlx4: 0000:05:00.0 15b3:1003 ConnectX-3:
  Supported: yes
stormnic-mlx4: 0000:05:00.0 15b3:1003 ConnectX-3:
  Start: bound; no firmware bring-up yet, releasing the NIC
```

See CLAUDE.md for the work plan.
