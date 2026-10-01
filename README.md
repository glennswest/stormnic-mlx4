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
bind on top of the SNP it installs on each Ethernet port. There is no PXE,
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
  (version 0x10; written in `src/main.rs`, since uefi-rs's binding refuses
  `Stop` with child handles). For each Ethernet port of a started ConnectX-3,
  a **child handle** with `EFI_DEVICE_PATH_PROTOCOL` (the PCI device path plus
  a MAC address node, IfType 1) and `EFI_SIMPLE_NETWORK_PROTOCOL` (revision
  0x00010000; `src/snp.rs`).
- **SNP:** MaxPacketSize 1500, MediaHeaderSize 14, receive filters unicast
  (the port MAC), broadcast and up to 16 multicast addresses (no promiscuous
  modes), MAC not changeable, `MediaPresent` from port-change events (or
  QUERY_PORT when MAP_EQ failed), `WaitForPacket`, `MCastIpToMac`. `Transmit`
  copies the frame, and `GetStatus` hands the caller's buffer back once the
  device has sent it. `Initialize` enables unicast and broadcast. `Statistics`,
  `StationAddress` and `NvData` return `UNSUPPORTED`. Frames the adapter loops
  back from the port's own MAC are dropped.
- **Consumes:** `EFI_PCI_IO_PROTOCOL` on the controller, opened `BY_DRIVER`
  while started and `BY_CHILD_CONTROLLER` by each child (a minimal binding in
  `src/pci.rs`: config-space reads/writes, BAR memory reads/writes,
  `AllocateBuffer`/`Map`/`Unmap`/`FreeBuffer` for DMA common buffers,
  `Attributes`, `GetBarAttributes` and `GetLocation`), the controller's device
  path, and tests for `EFI_SIMPLE_NETWORK_PROTOCOL` on it. While started it
  keeps memory space, bus master and dual-address-cycle enabled; `Stop` puts
  the original attributes back.
- **ExitBootServices:** an event stops the device's DMA before the OS takes
  the memory (spec 6.4): the teardown commands, silently and without freeing,
  then ownership released and bus master cleared.
- **BAR numbering:** PCI I/O's `BarIndex` for the UAR BAR (config 0x18) is
  not the same on every firmware (#15): EDK2 counts BAR registers (index 2),
  AMI Aptio 4 on the X9 blades counts BARs (index 1). `src/bars.rs` asks
  `GetBarAttributes` about both (then indexes 0–5) and takes the one whose
  base is the address in config space; the console line `UAR BAR (register
  2): BarIndex N` says which.
- **Port type:** Ethernet-only ports, and VPI ports forced to Ethernet whatever
  the firmware suggests (spec 5.1 has no command that sets the type; the port
  is driven with the Ethernet SET_PORT forms and Ethernet QPs). The console
  says `port N: type Ethernet (...)` and why.
- **Configuration:** none. No options, variables, ports or files are read; the
  PCI IDs it takes are compiled in (`DEVICES` in `src/main.rs`).

## Tests

`scripts/test-host.sh` builds and runs the host tests of the UEFI-independent
code against simulated firmware (`test/bars.rs`: EDK2 and AMI BAR numbering).
Run it on dev through `sc-build 'scripts/test-host.sh'`.

## How it ships

The output is `stormnic-mlx4.efi`. The approved shipping path (#8) is inside
stormbootx's `nic-drivers` golden, built from a pinned `STORMNIC_MLX4_REF`
(stormbootx#34, done). The `stormbootx-rustnic` media carries it without
`ipxe-hermon.efi`; the normal stormbootx media does not carry it. This
repository has no standalone component golden, and sc-build retains no
artifacts.

The pin decides what a boot tests: cef8dc5 (stormbootx#34) is the #1–#3 check
(bring-up, round trip, release). #4 needs the pin moved to the SNP commit or
later; the check is stormbootx printing `tcp4 : available` on server1 with only
this driver for the ConnectX-3. The master handles the media boot and console
capture.

## Status

Driver binding (#1), the firmware command interface (#2), the Ethernet data
path (#3) and the SNP (#4); hardware acceptance of each is pending on server1.
The image is an EFI boot-service driver (`build.rs` sets the PE subsystem;
`scripts/pe-subsystem.sh IMAGE` checks it is 11). Its entry point installs
`EFI_DRIVER_BINDING_PROTOCOL` and returns.

- `Supported` accepts a PCI function with vendor/device 15b3:1003 or 15b3:1007,
  unless it already has an SNP or another driver holds its PCI I/O `BY_DRIVER`:
  a platform's own driver always wins.
- `Start` brings the firmware up (spec sections 1–3, `src/fw.rs`): claim the
  ownership semaphore, reset the device and restore its config space,
  QUERY_FW, MAP_FA + RUN_FW, MOD_STAT_CFG, QUERY_DEV_CAP, QUERY_PORT per port,
  the ICM profile (a small one: about 2 MiB of host memory), SET_ICM_SIZE,
  MAP_ICM_AUX, MAP_ICM, INIT_HCA, QUERY_FUNC (SYS_EQS firmware only),
  QUERY_ADAPTER.
- Then the Ethernet data path (spec sections 4–6, `src/eth.rs`): an event
  queue (SW2HW_EQ, MAP_EQ), CONF_SPECIAL_QP, one physical memory region
  (SW2HW_MPT, its L_Key), and for every Ethernet port: receive and send CQs, a
  raw-Ethernet receive QP (256 × 2 KiB buffers) and send QP (128 TXBBs) taken
  RESET → INIT → RTR → RTS, SET_PORT MAC table and general settings (MTU 1526,
  and RQP_CALC with A0 steering), INIT_PORT, B0 steering entries for the port
  MAC and broadcast, SET_MCAST_FLTR off. It waits up to 5 s for link on every
  port, then installs the SNP children (`src/snp.rs`) and keeps the device.
- `Stop` uninstalls the children (when asked for them), then tears the device
  down: CLOSE_PORT, 2RST_QP, HW2SW_CQ per port, HW2SW_MPT, CONF_SPECIAL_QP 0,
  MAP_EQ unmap, HW2SW_EQ, CLOSE_HCA, UNMAP_ICM, UNMAP_ICM_AUX, UNMAP_FA,
  release ownership (1 s wait) and restore the PCI attributes. If a command
  times out or the teardown fails, it resets the device instead, and never
  frees memory the device may still use.
- `Start` failing at any step puts everything back the same way and returns
  `DEVICE_ERROR` (`UNSUPPORTED` when the NIC has no Ethernet port).
  Expect about 1.5 s per ConnectX-3 for bring-up (the reset takes 1 s) plus up
  to 5 s of link wait.

Source layout: `src/hcr.rs` (HCR protocol, spec 2), `src/fw.rs` (ownership,
reset, bring-up, profile, teardown, spec 1, 3 and 6.4), `src/eth.rs` (EQ, CQs,
QPs, MTT, memory region, port setup, steering, send/receive, link state, spec
4–6), `src/snp.rs` (the SNP and the child handles), `src/dma.rs` (DMA buffers,
big-endian accessors, page-list split, spec 3.2), `src/pci.rs`.

Every decision about a ConnectX-3 is printed to the console, including every
firmware command and its status, the QUERY_FW version and interface revision,
the raw QUERY_DEV_CAP bytes 0x10–0xa7 and the ICM layout, which is what the
spec's hardware checklist (section 7) asks for. A successful start looks like

```
stormnic-mlx4 0.2.0: driver binding installed (15b3:1003 ConnectX-3, 15b3:1007 ConnectX-3 Pro)
stormnic-mlx4: 0000:05:00.0 15b3:1003 ConnectX-3:
  Supported: yes
stormnic-mlx4: 0000:05:00.0 15b3:1003 ConnectX-3:
  Start: bound; bringing up the firmware
  ownership semaphore read 0: claimed
  reset: done (semaphore after 0 ms), config space restored
  QUERY_FW (0x0): ok
  firmware 2.x.y, command interface revision 3, ...
  ...
  INIT_HCA (0x0): ok
  ...
  firmware bring-up complete
  objects: PD 0x..., UAR page 0x80, EQ 0x..., first MTT ...
  ...
  port 1: MAC f4:52:14:84:b7:e0, MTU cap ..., RX QP ... CQ ..., TX QP ... CQ ..., B0 steering
  ...
  INIT_PORT (0x1): ok
  steering: unicast f4:52:14:84:b7:e0 -> QP ... (MCG entry ..., hash ...)
  steering: multicast ff:ff:ff:ff:ff:ff -> QP ... (MCG entry ..., hash ...)
  SET_MCAST_FLTR (0x1): ok
  port 1: up, link down
stormnic-mlx4: port 1: link up
  link up on every Ethernet port after 2300 ms
  port 1: SNP installed on a child handle, MAC f4:52:14:84:b7:e0, media present
  Start: 1 SNP child handle(s) installed
stormnic-mlx4: port 1 SNP: started
stormnic-mlx4: port 1 SNP: initialized, media present
stormnic-mlx4: port 1 SNP: receive filters 0x7, 1 multicast address(es)
stormnic-mlx4: port 1 tx: 342 bytes ff:ff:ff:ff:ff:ff <- f4:52:14:84:b7:e0 type 0800
stormnic-mlx4: port 1 rx: 342 bytes ff:ff:ff:ff:ff:ff <- ... type 0800
```

The first 16 frames each way per port are logged, one line each. A failed
step prints the command with its status (`status 0x03, bad parameter`), a
timeout also prints the catastrophic error buffer, and `Start` ends with
`Start: firmware bring-up failed, releasing the NIC` or `Start: data path
bring-up failed, releasing the NIC`. Per port: `port N: not Ethernet (...);
skipped`, `port N: no link after 5 s; reported as no media`, link changes
(`stormnic-mlx4: port N: link up/down`), error completions (`rx:`/`tx: error
completion, syndrome ...`) and EQ events (CQ and QP errors). Other outcomes:
`Supported: already has an SNP, leaving it to the platform's driver`,
`Supported: no, PCI I/O is held (<status>)`, `Start: cannot claim PCI I/O
(<status>)`, and `ownership semaphore reads <v>: another function or driver
owns the device; leaving it`. Handles that are not a ConnectX-3 are rejected
silently (the firmware offers every handle in the system). `Stop` prints
`stormnic-mlx4: Stop: ...` with the children left or the teardown. The
ExitBootServices handler prints nothing.

See CLAUDE.md for the work plan.
