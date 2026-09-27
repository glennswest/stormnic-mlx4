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
sc-build 'cargo build --release --target x86_64-unknown-uefi'
```

## Status

Scaffold: the entry point logs and returns `UNSUPPORTED`. See CLAUDE.md for the
work plan.
