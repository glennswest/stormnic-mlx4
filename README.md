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

Driver binding (#1), the firmware command interface (#2) and the Ethernet
data path (#3); no SNP (#4) yet. The image is an EFI boot-service driver (`build.rs` sets
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
  QUERY_ADAPTER.
- Then the Ethernet data path (spec sections 4–6, `src/eth.rs`): an event
  queue (SW2HW_EQ, MAP_EQ), CONF_SPECIAL_QP, one physical memory region
  (SW2HW_MPT, its L_Key), and for each Ethernet port in turn: receive and send
  CQs, a raw-Ethernet receive QP (256 × 2 KiB buffers) and send QP (128 TXBBs)
  taken RESET → INIT → RTR → RTS, SET_PORT MAC table and general settings
  (MTU 1526, and RQP_CALC with A0 steering), INIT_PORT, B0 steering entries
  for the port MAC and broadcast, SET_MCAST_FLTR off. It waits up to 10 s for
  link, broadcasts a DHCPDISCOVER (and an ARP probe for an address it has seen
  on the wire), logs every frame for up to 6 s, and reports whether a reply
  came back. The port is then closed again (CLOSE_PORT, 2RST_QP, HW2SW_CQ).
- Last, the teardown: HW2SW_MPT, CONF_SPECIAL_QP 0, MAP_EQ unmap, HW2SW_EQ,
  CLOSE_HCA, UNMAP_ICM, UNMAP_ICM_AUX, UNMAP_FA, release ownership (1 s wait)
  and restore the PCI attributes. If a command times out or the teardown
  fails, it resets the device instead, and never frees memory the device may
  still use (queue and frame buffers are freed only after UNMAP_FA or a
  reset). It then returns `UNSUPPORTED`: without an SNP, holding the device
  would only keep another driver off it.
  Expect about 2.5 s per ConnectX-3 (reset 1 s, ownership release 1 s), plus
  the link wait and up to 6 s of listening per Ethernet port with link.

Source layout: `src/hcr.rs` (HCR protocol, spec 2), `src/fw.rs` (ownership,
reset, bring-up, profile, teardown, spec 1 and 3), `src/eth.rs` (EQ, CQs, QPs,
MTT, memory region, port setup, steering, send/receive and the round-trip
check, spec 4–6), `src/dma.rs` (DMA buffers, big-endian accessors, page-list
split, spec 3.2), `src/pci.rs`.

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
  firmware bring-up complete
  objects: PD 0x..., UAR page 0x80, EQ 0x..., first MTT ...
  SW2HW_EQ (0x..): ok
  ...
  port 1: MAC f4:52:14:84:b7:e0, MTU cap ..., RX QP ... CQ ..., TX QP ... CQ ..., B0 steering
  ...
  INIT_PORT (0x1): ok
  steering: unicast f4:52:14:84:b7:e0 -> QP ... (MCG entry ..., hash ...)
  steering: multicast ff:ff:ff:ff:ff:ff -> QP ... (MCG entry ..., hash ...)
  port 1: link up after 2300 ms
  port 1: broadcasting DHCPDISCOVER (xid 0x........)
  port 1 rx: 342 bytes ff:ff:ff:ff:ff:ff <- ... type 0800
  port 1: broadcast round trip ok: DHCPOFFER of a.b.c.d from ... (server a.b.c.d)
  port 1: sent 1, received 5 (4 broadcast, 0 own frames looped back), dropped 0, send errors 0
  CLOSE_PORT (0x1): ok
  ...
  port 1: closed, QPs reset, CQs returned
  data path check complete; no SNP yet (#4), tearing it down
  HW2SW_MPT (0x..): ok
  ...
  CLOSE_HCA (0x0): ok
  ...
  UNMAP_FA (0x0): ok
  firmware stopped, memory returned
  ownership released
  Start: firmware and data path check done; no SNP yet (#4), releasing the NIC
```

A failed step prints the command with its status (`status 0x03, bad
parameter`), a timeout also prints the catastrophic error buffer, and `Start`
ends with `Start: firmware or data path check failed, releasing the NIC`. Per
port: `port N: not Ethernet (...); skipped`, `port N: no link after 10 s; not
tested`, `port N: no reply to the broadcast within 6 s`, error completions
(`rx:`/`tx: error completion, syndrome ...`) and EQ events (`event: port N link
down/active`, CQ and QP errors). Other outcomes:
`Supported: already has an SNP, leaving it to the platform's driver`,
`Supported: no, PCI I/O is held (<status>)`, `Start: cannot claim PCI I/O
(<status>)`, and `ownership semaphore reads <v>: another function or driver
owns the device; leaving it`. Handles that are not a ConnectX-3 are rejected
silently (the firmware offers every handle in the system). `Stop` prints
`stormnic-mlx4: Stop` and releases nothing, since `Start` never keeps the
device.

See CLAUDE.md for the work plan.
