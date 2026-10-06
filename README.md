# stormnic-mlx4

A UEFI driver, in Rust (`no_std`), that gives firmware an
`EFI_SIMPLE_NETWORK_PROTOCOL` for **Mellanox ConnectX-3 / ConnectX-3 Pro Ethernet (the Linux `mlx4` family)**.

## Why it exists

stormbootx boots a machine over NVMe/TCP with its own TCP/IP stack (smoltcp)
running on each NIC's `EFI_SIMPLE_NETWORK_PROTOCOL` (stormbootx#56); the
firmware's `EFI_TCP4` is not used. That needs a UEFI driver for the NIC. The
Supermicro X9 blades (server1–8) have only legacy option ROMs for their Intel
10G and ConnectX-3 ports, so no SNP exists (stormbootx#26).

stormbootx runs one `ConnectController` pass so the platform's own drivers
claim their NICs, then loads and starts every `*.efi` in `\stormboot\drivers`
on its boot media, then connects every handle again and opens each SNP
`EXCLUSIVE` for smoltcp (DHCP, ARP, TCP). Its console then prints
`tcp4 : smoltcp over SNP (<nics>)`. This driver is one of those files and
provides the SNP on each Ethernet port; a firmware MNP/IP4/TCP4 stack can bind
on top of it as well. There is no PXE, no DHCP boot and no network code of its
own above the link layer.

The interim driver was iPXE's `ipxe-hermon.efi` (GPL-2 C, built from pinned
source by stormbootx's `scripts/build-nic-drivers.sh`). It hangs server1's boot,
so stormbootx builds it only on request (`IPXE_DRIVERS="intelx hermon"`), and
the rustnic media carries no iPXE at all. This crate replaces it (stormbootx#27,
#5).

## Hardware

- PCI IDs: 15b3:1003 (ConnectX-3), 15b3:1007 (ConnectX-3 Pro); the X9 blades carry 15b3:1003
- Reference: [`docs/spec/connectx3.md`](docs/spec/connectx3.md), the ConnectX-3
  programming specification this driver is written from

**Written from `docs/spec/connectx3.md` only.** The public ConnectX-3 PRM does
not exist (NVIDIA supplies it under a support contract), so an independent
agent wrote that spec from the OpenIB.org BSD option of the dual-licensed
Linux/FreeBSD mlx4 sources (#10; module EEPROM, PTYS and speed/autoneg added in #20); [`NOTICE`](NOTICE) carries their notice. The
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

A verbose image, which prints the whole bring-up trace on every boot, is
`--features verbose` on the same command
(`sc-build 'cargo build --locked --release --target x86_64-unknown-uefi --features verbose'`).
The shipped image is the default build. Use the variable to make it verbose.

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
  back from the port's own MAC are dropped; the first is logged (spec 7 item 14).
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
- **Configuration:** one switch, console output only (#16, see "Console
  output"): the EFI variable `StormnicVerbose` under vendor GUID
  `ce1479a2-eab9-4176-b0ad-c909ea5b8e0b` (shared by every stormnic driver),
  read once at the entry point; a first data byte other than 0 turns the
  bring-up trace on. The `verbose` build feature does the same at build time.
  Nothing else is read; the PCI IDs it takes are compiled in (`DEVICES` in
  `src/main.rs`).

## Tests

`scripts/test-host.sh` builds and runs the host tests of the UEFI-independent
code against simulated firmware (`test/bars.rs`: EDK2 and AMI BAR numbering) and
the link-diagnostics decoding (`test/module.rs`: module EEPROM fields, MAD
status codes, request splitting, PTYS link modes; spec 5.11–5.12) and the
quiet console's replay ring (`test/trace.rs`, #16).
Run it on dev through `sc-build 'scripts/test-host.sh'`.

## How it ships

The output is `stormnic-mlx4.efi`. The approved shipping path (#8) is inside
stormbootx's `nic-drivers` build, from a pinned `STORMNIC_MLX4_REF` in
stormbootx's `scripts/build-nic-drivers.sh` (stormbootx#34). The
`stormbootx-rustnic` media (`STORMNIC_ON_MEDIA="ixgbe mlx4"`) carries it with
no iPXE; the normal stormbootx media does not carry it. This repository has no
standalone component golden, and sc-build retains no artifacts.

The pin is `0e50017` (v0.2.3: the link and module lines, and the own-frame line
for spec 7 item 14; stormbootx#66). stormbootx#104 asks for v0.2.5 (`04e7d2c`:
the module EEPROM, PTYS and speed-control lines of #17, and the quiet console of
#16; boot verbose for the hardware checks). A boot of the rustnic media is the hardware
check: stormbootx prints `tcp4 : smoltcp over SNP (...)` with this driver's
`port N SNP: initialized` and `port N rx:` lines behind it. The master handles
the media boot and console capture.

## Status

Driver binding (#1), the firmware command interface (#2), the Ethernet data
path (#3) and the SNP (#4) are done and were checked on metal on 2026-10-01:
on server3 (X9, AMI Aptio 4, v0.2.1) every command through INIT_HCA, port
bring-up, steering, the SNP and a transmitted DHCP discover; on server1 the
ConnectX-3 reached link at 10G. From 2026-10-02, rustnic boots of server3 and
server8 at v0.2.1 receive through this driver. They get a DHCP lease, and on
server3 also claim and attach the boot image over it. Spec section 7.1 records
hardware checks 1–13 from those boots (#18). Item 14 waits for a boot at v0.2.3
(#21), items 15–16 are #12, and items 17–18 (module EEPROM, PTYS) wait for a boot
at v0.2.4 (#17).
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
  port, prints the link diagnostics of #17 (`src/diag.rs`: the module EEPROM
  through MAD_IFC, spec 5.11, and a read-only ACCESS_REG PTYS query, spec 5.12),
  then installs the SNP children (`src/snp.rs`) and keeps the device. It never
  forces a speed: the PTYS write of spec 5.13 is not used.
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
4–6), `src/console.rs` and `src/trace.rs` (quiet console, verbose switch and
replay ring, #16), `src/diag.rs` and `src/module.rs` (link diagnostics: module EEPROM and
PTYS, spec 5.11–5.13), `src/snp.rs` (the SNP and the child handles), `src/dma.rs` (DMA buffers,
big-endian accessors, page-list split, spec 3.2), `src/pci.rs`.

### Console output

By default (#16) the console gets **one line per port** when `Start`
succeeds, plus every warning and error:

```
stormnic-mlx4 0.2.5: 0000:05:00.0 15b3:1003 ConnectX-3 port 1: MAC f4:52:14:84:b7:e0, link up 10G XFI, SNP installed
```

(`no link` in place of `link up …` when the 5 s link wait ran out.)

**Verbose** prints the full trace: every decision about a ConnectX-3, every
firmware command and its status, the QUERY_FW version and interface revision,
the raw QUERY_DEV_CAP bytes 0x10–0xa7 and the ICM layout (what the spec's
hardware checklist, section 7, asks for), the SNP calls, the first 16 frames
each way per port, and the #17 link diagnostics on a port with link. Turn it
on without a rebuild by setting the variable before the driver loads.
stormbootx does this from its config. From the UEFI shell:

```
setvar StormnicVerbose -guid ce1479a2-eab9-4176-b0ad-c909ea5b8e0b -bs =01
```

or build with `--features verbose`. The code is `src/console.rs`: `say!`
always prints, `trace!` prints only when verbose, `note!(loud, …)` picks one,
and `alarm!` marks a failure. When quiet, the trace lines are not lost
outright. The last 16 are kept (`src/trace.rs`), and a failure prints them
first (`stormnic-mlx4: the N step(s) before the failure below (M earlier not
kept):`), then the failure. So a quiet console still shows the failing
command, its status and the steps before it.

**Always printed** (quiet or verbose):
- the per-port line above;
- `stormnic-mlx4: LOC 15b3:DDDD NAME: already has an SNP, leaving it to the
  platform's driver`, `…: not taken, PCI I/O is held (<status>)` and
  `stormnic-mlx4: Start: cannot claim PCI I/O (<status>)`;
- a failed firmware command (`  OP (in_mod): status 0x03, bad parameter`,
  `timed out after 60 s` with the catastrophic error buffer) and every other
  bring-up failure, after the replayed steps. `Start` then ends with
  `stormnic-mlx4: LOC 15b3:DDDD NAME: Start: firmware bring-up failed,
  releasing the NIC` (or `data path bring-up failed`, `no SNP installed`, `no
  Ethernet port`);
- `stormnic-mlx4: port N: no link after 5 s; reported as no media`, and for
  such a port what QUERY_PORT reports (speed code, autonegotiation enabled and
  complete, transceiver type, vendor OUI, wavelength and code, spec 3.5) and
  the #17 link diagnostics below;
- link changes (`stormnic-mlx4: port N: link up/down`), and at link-down the
  QUERY_PORT line and the link diagnostics;
- the first own frame the adapter loops back (`port N rx: own frame looped
  back (... bytes to ...), dropped (5.10)`, spec 7 item 14);
- error completions (`port N rx:`/`tx: error completion, syndrome ...`), a
  failed TX doorbell, EQ errors (`event: CQ …`, `event: QP …`, `event: local
  catastrophic error`), `MAP_EQ failed`, `port N: not Ethernet-capable (...);
  skipped`, `QUERY_PORT failed`, teardown failures (`teardown by command
  failed; resetting the device instead`), and `ownership semaphore reads <v>:
  another function or driver owns the device; leaving it`.

**Link diagnostics** (#17, spec 5.11–5.13): `port N: module EEPROM:` with the
MAD_IFC HCR status, the MAD status (a cable-info error such as `0x0400 ... no
EEPROM (passive copper cable)` is not a fault on a DAC) and the identifier,
then for an SFP or QSFP the raw bytes and their SFF labels (passive or active
cable, length, vendor, part, serial; spec 5.11.5). `port N: PTYS:` gives the
supported, advertised, operating and partner link modes when the firmware
answers ACCESS_REG. Without QUERY_DEV_CAP 0x7a bit 5 that is one read-only
query for spec 7 item 18, not repeated once refused (spec 5.12). Then `speed
control:` says whether the card offers a forced speed (spec 5.13); the blades'
firmware 2.30.8000 does not. These lines are printed after the link wait and at
every link-down. They are always printed for a port without link, and only in
the verbose trace for one with link. **For spec 7 items 17–18 on a port with
link, boot verbose.**

A verbose start looks like

```
stormnic-mlx4 0.2.5: driver binding installed (15b3:1003 ConnectX-3, 15b3:1007 ConnectX-3 Pro)
stormnic-mlx4: 0000:05:00.0 15b3:1003 ConnectX-3: Supported
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
  port 1: link up, speed code 0x01 (10G XFI), autoneg enabled, complete
  port 1: module: transceiver type 0x.., vendor OUI ..:..:.., wavelength ..., code 0x...
  port 1: module EEPROM: HCR status 0, MAD status 0, identifier 0x03 (SFP/SFP+)
  port 1: module EEPROM: eeprom 00: 03 04 21 ...
  ...
  port 1: module EEPROM: SFP/SFP+: passive copper cable (byte 8 0x04), connector 0x21 (copper pigtail)
  port 1: module EEPROM: length 3 m (byte 18), nominal rate 10300 MBd (byte 12), ...
  port 1: module EEPROM: vendor "..." OUI ..:..:.., part "..." rev "..", serial "..."
  port 1: module EEPROM: cable compliance .. .. (bytes 60-61), diagnostics ... (byte 92 0x..)
  port 1: PTYS: not offered (QUERY_DEV_CAP 0x7a bit 5, ETH_PROT_CTRL, is 0); one read-only query for spec 7 item 18
  port 1: PTYS: ... (the firmware's answer, or its status)
  speed control: not offered (ETH_PROT_CTRL 0): no forced speed or autoneg setting on this card (5.13); set the switch port
  port 1: SNP installed on a child handle, MAC f4:52:14:84:b7:e0, media present
  Start: 1 SNP child handle(s) installed
stormnic-mlx4 0.2.5: 0000:05:00.0 15b3:1003 ConnectX-3 port 1: MAC f4:52:14:84:b7:e0, link up 10G XFI, SNP installed
stormnic-mlx4: port 1 SNP: started
stormnic-mlx4: port 1 SNP: initialized, media present
stormnic-mlx4: port 1 SNP: receive filters 0x7, 1 multicast address(es)
stormnic-mlx4: port 1 tx: 342 bytes ff:ff:ff:ff:ff:ff <- f4:52:14:84:b7:e0 type 0800
stormnic-mlx4: port 1 rx: 342 bytes ff:ff:ff:ff:ff:ff <- ... type 0800
```

(The PTYS answer on the blades is not known yet; that is spec 7 item 18.)
`Stop` traces `stormnic-mlx4: Stop: ...` with the children left or the
teardown. Handles that are not a ConnectX-3 are rejected silently (the
firmware offers every handle in the system). The ExitBootServices handler
prints nothing.

See CLAUDE.md for the work plan.
