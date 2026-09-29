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
  `src/pci.rs`: config-space reads/writes, BAR 0 memory reads/writes,
  `AllocateBuffer`/`Map`/`Unmap`/`FreeBuffer` for DMA common buffers,
  `Attributes`, `GetBarAttributes` and `GetLocation`), and tests for
  `EFI_SIMPLE_NETWORK_PROTOCOL` on it. While `Start` runs it enables memory
  space, bus master and dual-address-cycle, and puts the original attributes
  back before returning.
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
`\stormboot\drivers`. The #1/#2 hardware check requires this driver on the media
without `ipxe-hermon.efi` (stormbootx's `stormbootx-rustnic` media,
stormbootx#45); `ipxe-intelx.efi` can remain because this driver
releases the ConnectX-3. The master handles the media boot and console capture.
Acceptance requires the install log and `Supported: yes` / `Start: bound` for
server1's `0000:05:00.0`; passing the build alone does not complete #1.

## Status

Driver binding (#1) and the firmware command interface (#2); no data path
(#3) or SNP (#4) yet. The image is an EFI boot-service driver (`build.rs` sets
the PE subsystem; `scripts/pe-subsystem.sh IMAGE` checks it is 11). Its entry
point installs `EFI_DRIVER_BINDING_PROTOCOL` and returns.

- `Supported` accepts a PCI function with vendor/device 15b3:1003 or 15b3:1007,
  unless it already has an SNP or another driver holds its PCI I/O `BY_DRIVER`:
  a platform's own driver always wins.
- `Start` brings the firmware up and takes it down again (spec sections 1–3):
  claim the ownership semaphore, reset the device and restore its config
  space, QUERY_FW, MAP_FA + RUN_FW, MOD_STAT_CFG, QUERY_DEV_CAP, QUERY_PORT per
  port, the ICM profile (a small one: about 2 MiB of host memory), SET_ICM_SIZE,
  MAP_ICM_AUX, MAP_ICM, INIT_HCA, QUERY_FUNC (SYS_EQS firmware only),
  QUERY_ADAPTER; then CLOSE_HCA, UNMAP_ICM, UNMAP_ICM_AUX, UNMAP_FA, release
  ownership (1 s wait) and restore the PCI attributes. If a command times out
  or the teardown fails, it resets the device instead, and never frees memory
  the firmware may still use. It then returns `UNSUPPORTED`: without a data
  path, holding the device would only keep another driver off it.
  Expect about 2.5 s per ConnectX-3 (reset 1 s, ownership release 1 s).

Source layout: `src/hcr.rs` (HCR protocol, spec 2), `src/fw.rs` (ownership,
reset, bring-up, profile, teardown, spec 1 and 3), `src/dma.rs` (DMA buffers,
big-endian accessors, page-list split, spec 3.2), `src/pci.rs`.

Every decision about a ConnectX-3 is printed to the console, including every
firmware command and its status, the QUERY_FW version and interface revision,
the raw QUERY_DEV_CAP bytes 0x10–0xa7 and the ICM layout, which is what the
spec's hardware checklist (section 7) asks for. A successful run looks like

```
stormnic-mlx4 0.1.0: driver binding installed (15b3:1003 ConnectX-3, 15b3:1007 ConnectX-3 Pro)
stormnic-mlx4: 0000:05:00.0 15b3:1003 ConnectX-3:
  Supported: yes
stormnic-mlx4: 0000:05:00.0 15b3:1003 ConnectX-3:
  Start: bound; bringing up the firmware
  ownership semaphore read 0: claimed
  reset: done (semaphore after 0 ms), config space restored
  HCR: status dword 00000000 after reset (GO 0, T 0); first toggle 1
  QUERY_FW (0x0): ok
  firmware 2.x.y, command interface revision 3, ...
  ...
  INIT_HCA (0x0): ok
  ...
  firmware bring-up complete; no data path yet (#3), tearing it down
  CLOSE_HCA (0x0): ok
  ...
  UNMAP_FA (0x0): ok
  firmware stopped, memory returned
  ownership released
  Start: firmware check passed; no data path yet (#3), releasing the NIC
```

A failed step prints the command with its status (`status 0x03, bad
parameter`), a timeout also prints the catastrophic error buffer, and `Start`
ends with `Start: firmware check failed, releasing the NIC`. Other outcomes:
`Supported: already has an SNP, leaving it to the platform's driver`,
`Supported: no, PCI I/O is held (<status>)`, `Start: cannot claim PCI I/O
(<status>)`, and `ownership semaphore reads <v>: another function or driver
owns the device; leaving it`. Handles that are not a ConnectX-3 are rejected
silently (the firmware offers every handle in the system). `Stop` prints
`stormnic-mlx4: Stop` and releases nothing, since `Start` never keeps the
device.

See CLAUDE.md for the work plan.
