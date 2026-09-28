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

stormbootx runs one `ConnectController` pass so the platform's own drivers
claim their NICs, then loads and starts every `*.efi` in `\stormboot\drivers`
on its boot media, then connects every handle again before it looks for TCP4.
This driver is one of those files. The firmware's MNP, IP4 and TCP4 drivers are
meant to bind on top of the SNP it installs (once #2–#4 exist). There is no PXE,
no DHCP boot and no network code of its own above the link layer.

The interim driver is iPXE's `ipxe-hermon.efi` (GPL-2 C, built from pinned
source by stormbootx's `scripts/build-nic-drivers.sh`). It hangs server1's boot,
so stormbootx now builds it only on request (`IPXE_DRIVERS="intelx hermon"`).
This crate replaces it (stormbootx#27, #5).

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

## Interfaces and configuration

- **Installs:** `EFI_DRIVER_BINDING_PROTOCOL` on its own image handle
  (binding version 1, via `uefi::driver::install`). Nothing else yet; no SNP.
- **Consumes:** `EFI_PCI_IO_PROTOCOL` on the controller (a minimal binding in
  `src/pci.rs`: config-space dword reads and `GetLocation`), and tests for
  `EFI_SIMPLE_NETWORK_PROTOCOL` on it.
- **Configuration:** none. No options, variables, ports or files are read; the
  PCI IDs it takes are compiled in (`DEVICES` in `src/main.rs`).

## How it ships

One file, `stormnic-mlx4.efi`, from the build above. It is not a stormcentral
component and has no golden or release artifact: to use it, copy it into the
directory given to stormbootx's `scripts/build-boot-agent.sh --iso --drivers DIR`,
which lays it in `\stormboot\drivers` on the media. Wiring it into the
stormbootx media build in place of `ipxe-hermon.efi` is #5 / stormbootx#27.

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

The other outcomes print `Supported: already has an SNP, leaving it to the
platform's driver`, `Supported: no, PCI I/O is held (<status>)`, or
`Start: cannot claim PCI I/O (<status>)`. Handles that are not a ConnectX-3 are
rejected silently (the firmware offers every handle in the system).
`Stop` prints `stormnic-mlx4: Stop` and releases nothing, since `Start` never
keeps the device.

See CLAUDE.md for the work plan.
