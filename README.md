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
- Reference: [`docs/spec/connectx3.md`](docs/spec/connectx3.md), the ConnectX-3
  programming specification this driver is written from

**Written from `docs/spec/connectx3.md` only.** The public ConnectX-3 PRM does
not exist (NVIDIA supplies it under a support contract), so an independent
agent wrote that spec from the OpenIB.org BSD option of the dual-licensed
Linux/FreeBSD mlx4 sources (#10); [`NOTICE`](NOTICE) carries their notice. The
driver is implemented from the spec, not from those sources, and nothing comes
from iPXE's hermon (GPL). The crate is MIT.

## Build

`x86_64-unknown-uefi`, built on dev with `sc-build` after pushing:

```bash
sc-build 'cargo build --locked --release --target x86_64-unknown-uefi && scripts/pe-subsystem.sh "${CARGO_TARGET_DIR:-target}"/x86_64-unknown-uefi/release/stormnic-mlx4.efi'
```

`Cargo.lock` is committed and the build uses `--locked`, so a pinned commit
(stormbootx's `STORMNIC_MLX4_REF`) always builds against the same `uefi` /
`uefi-raw` versions (#9). To move a dependency, update the lock on purpose
(`sc-build 'cargo update -p uefi && cat Cargo.lock'`, then commit the result)
and note it in the changelog.

## Interfaces and configuration

- **Installs:** `EFI_DRIVER_BINDING_PROTOCOL` on its own image handle
  (binding version 1, via `uefi::driver::install`). Nothing else yet; no SNP.
- **Consumes:** `EFI_PCI_IO_PROTOCOL` on the controller (a minimal binding in
  `src/pci.rs`: config-space dword reads and `GetLocation`), and tests for
  `EFI_SIMPLE_NETWORK_PROTOCOL` on it.
- **Configuration:** none. No options, variables, ports or files are read; the
  PCI IDs it takes are compiled in (`DEVICES` in `src/main.rs`).

## How it ships

The output is `stormnic-mlx4.efi`. The approved shipping path (#8) is inside
stormbootx's `nic-drivers` golden, built from a pinned `STORMNIC_MLX4_REF`.
That media integration is tracked in
[stormbootx#34](https://github.com/glennswest/stormbootx/issues/34) and is still
pending: #29 closed with ixgbe integration only. This repository has no
standalone component golden, and sc-build retains no artifacts.

For manual media assembly, stormbootx's
`scripts/build-boot-agent.sh --iso --drivers DIR` lays the supplied drivers in
`\stormboot\drivers`. The #1 hardware check requires this driver on the media
without `ipxe-hermon.efi`; `ipxe-intelx.efi` can remain because this scaffold
releases the ConnectX-3. The master handles the media boot and console capture.
Acceptance requires the install log and `Supported: yes` / `Start: bound` for
server1's `0000:05:00.0`; passing the build alone does not complete #1.

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
