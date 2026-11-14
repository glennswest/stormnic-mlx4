# CLAUDE.md — stormnic-mlx4

A `no_std` UEFI driver giving firmware an `EFI_SIMPLE_NETWORK_PROTOCOL` for
Mellanox ConnectX-3 / ConnectX-3 Pro Ethernet (the Linux `mlx4` family). It's loaded by stormbootx from `\stormboot\drivers` on its boot
media, on machines whose firmware has the TCP/IP stack but no UEFI driver for
the NIC (the Supermicro X9 blades, stormbootx#26). Read README.md first.

Read the cross-project rules in `../CLAUDE.md` first. In particular, **build
with `sc-build` after pushing, never on this VM and never as root**, and
scratch files go in `tmp/`.

## Rules for this crate

- **Written from `docs/spec/connectx3.md` only** (owner, #2, 2026-09-29). That spec was
  written by an independent agent from the BSD option of the dual-licensed Linux/FreeBSD
  mlx4 sources (#10); `NOTICE` carries their BSD notice and must stay. The driver author
  does not read those sources or iPXE's hermon (GPL); anything the spec does not settle is
  a question for the spec (#10), not a look at another driver. Cite spec sections in code.
- It is a *driver*: the binary is an EFI boot-service driver
  (`/subsystem:efi_boot_service_driver`), not an application, and it
  installs `EFI_DRIVER_BINDING_PROTOCOL` so the firmware's `ConnectController`
  binds it. stormbootx loads it and then connects controllers.
- A platform's own driver must win: if the NIC already has an SNP, do nothing.
- Everything the driver does is logged, through `src/console.rs` (#16): `trace!` for the bring-up trace (printed
  only when verbose: `StormnicVerbose` or `--features verbose`; else kept in a 16-line ring a failure replays),
  `say!` for the one line per port and every warning or error, `alarm!` for failures. Never `uefi::println!`
  directly. The only way to debug it on the blades is the SOL capture on stormcentral
  (`/var/lib/stormcentral/console/serverN/sol.log`); boot verbose for a hardware check.

## Build

```bash
sc-build 'cargo build --locked --release --target x86_64-unknown-uefi && scripts/pe-subsystem.sh "${CARGO_TARGET_DIR:-target}"/x86_64-unknown-uefi/release/stormnic-mlx4.efi'
```

`Cargo.lock` is committed (#9); keep `--locked`, and change dependencies only by a deliberate lock update.
The image is byte-reproducible (#13): keep `.cargo/config.toml` (rustc wrapper `scripts/rustc-remap.sh`) and
`/Brepro` in `build.rs`, and check with `sc-build scripts/repro-check.sh` after a build or dependency change.
The second half checks the image is a boot-service driver (PE subsystem 11).
Each sc-build job gets its own drive, deleted afterwards, so the image does not
survive the job; `${CARGO_TARGET_DIR:-target}` finds it wherever the job puts
the target dir.

## How it ships

No standalone component golden. The owner approved shipping inside stormbootx's
`nic-drivers` build from a pinned `STORMNIC_MLX4_REF` (#8). stormbootx#34 did
the mlx4 integration: `scripts/build-nic-drivers.sh` builds this crate at the
pin, and the `stormbootx-rustnic` media (`STORMNIC_DRIVERS`, default `ixgbe mlx4`)
carries it. stormbootx ships no iPXE on any medium (stormbootx#91). The pin was `cf37f8b` (v0.2.1) as of 2026-10-04;
stormbootx#66 pinned v0.2.3 (`0e50017`); stormbootx#104 asks for v0.2.5 (`04e7d2c`; first asked for v0.2.4). `--drivers DIR` is the manual media assembly
interface, not a way to retain an sc-build artifact. sc-build keeps no image.
The media/golden work belongs to stormbootx; do not create a persistent build
checkout or copy artifacts out of sc-build.

No ports or APIs; the interfaces are the driver binding, the SNP children, and one console switch, the EFI
variable `StormnicVerbose` (#16; README, "Interfaces and configuration").

## Test

Host tests: `sc-build 'scripts/test-host.sh'` (`test/bars.rs`).

Hardware: put the built `.efi` in a stormbootx ISO's `\stormboot\drivers`
(`scripts/build-boot-agent.sh --iso --drivers DIR` in stormbootx), or use the rustnic golden at the current pin. Boot a blade from
the virtual CD, which the stormcentral minismbd serves as `\boot\stormbootx.iso`.
**The blade boot is the master's job, not an owner decision** (master on #1,
2026-09-30): ask the master on the issue, never with `needs-owner`;
stormcentral#237 is to settle a proper hardware-boot request. stormbootx does
not use the firmware's TCP4: its console line is `tcp4 : smoltcp over SNP
(<nics>)`, never `tcp4 : available`.

## Version

`Cargo.toml` → `package.version`. Current: `v0.2.8`.

## Work plan

Done (history in git and CHANGELOG.md):

- [x] #1 driver binding, #2 firmware command interface, #3 Ethernet data path, #4 SNP on a child handle per
  Ethernet port. Closed 2026-10-01 on server3 (X9, `golden-stormbootx-rustnic-bde9ae3a566c4d7d`, v0.2.1):
  binding, every command through INIT_HCA, SET_PORT/INIT_PORT, steering, SNP installed, TX of a DHCP discover;
  and server1 link-ok at 10G (DAC on the CRS326 sfp-sfpplus1, fixed 10G). RX and a lease were not seen (#18).
- [x] #9 `Cargo.lock` committed, builds `--locked`.
- [x] #5 `ipxe-hermon.efi` retired: stormbootx#91 (closed 2026-10-06) removed iPXE from every medium. The condition (a
  ConnectX-3 boothost claim with this driver alone) was met by server3's rustnic boots (§7.1).
- [x] #10 `docs/spec/connectx3.md` (the only source; BSD notice in `NOTICE`). #20 added 5.11–5.13 (EEPROM, PTYS, speed).
- [x] #18 RX, a DHCP lease and an NVMe/TCP claim through the ConnectX-3 (server3 and server8 rustnic boots, v0.2.1,
  2026-10-02..05). Spec §7.1 records HW-checks 1–13; item 14 split to #21 (v0.2.3).
- [x] #13 byte-reproducible image (v0.2.6, `e5b9892`, 2026-11-07): rustc wrapper `scripts/rustc-remap.sh` remaps
  build paths, `/Brepro`, and `/DEBUG:NONE` (lld's PDB GUID followed the build directory). `scripts/repro-check.sh`
  passed in sc-build: two directories, two `CARGO_HOME`s, identical images (sha256 `ce07585b…`, 105472 bytes).
- [x] #15 UAR `BarIndex` from `GetBarAttributes` (v0.2.1), VPI ports forced to Ethernet, link/module
  diagnostics at link wait and link changes (v0.2.2). v0.2.2 pin requested in stormbootx#66.

Open:

- [x] #21 §7 item 14 (own-frame loopback): **no loopback** with LB_SRC_CHK 1. Recorded in spec §7.1 2026-11-07 from 6
  v0.2.5 boots (server1 ×4, server7, server8) that leased through the ConnectX-3; no `own frame looped back` line.
- [ ] #12 §7 items 15 and 16, recorded in spec §7.1 (2026-10-06, 34 SOL captures, server1/3/4/7/8). **16 done**: 256
  reserved MPTs → MPT 0x100, L_Key 0x00010000, no collision. **15, orderly teardown: answered 2026-11-07.** The image
  cmdline now has `console=ttyS1` (stormcos#220 is still open, but the kernel reaches the SOL): ~40 v0.2.5 boots on
  server1/2/7/8 show `mlx4_core` probing cleanly after the hand-off; `STARTING KERNEL` → kernel clock ≤ 1.0 s
  (`.log.times`, 36 boots). **15, bus-master-only fallback (6.4 HW-CHECK): in progress.** No boot exercised it. Branch
  `test/ebs-fallback` (never merged) makes ExitBootServices skip the command sequence; a rustnic boot at that ref
  (master, `wait-hardware`) must show `mlx4_core` probing cleanly. Then record it in §7.1 and close #12.
- [ ] #17 DAC link diagnostics, now specified (#20 done 2026-10-06: spec 5.11–5.13, HW-checks 17–19). Item 1
  (module EEPROM via MAD_IFC 0xFF60, spec 5.11) can be built. Items 2–3 need PTYS, gated on QUERY_DEV_CAP 0x7a
  bit 5, which the blades' firmware 2.30.8000 leaves 0 (spec 5.13): log that, no speed setting (stormbootx#80).
  **Built in v0.2.4 (`32083cd`, 2026-10-06):** `src/diag.rs`/`src/module.rs` print the module EEPROM (5.11) and a
  read-only PTYS query (5.12) after the link wait and at every link-down, plus a `speed control:` line; no PTYS
  write (item 3 is not offered on the blades, 5.13). sc-build passes (subsystem 11, host tests 9 + 8). Pin asked in
  stormbootx#104 — pinned v0.2.5 (`04e7d2c`, stormbootx 10e9377). Checked 2026-11-07: quiet v0.2.5 boots on
  server1/2/8 (stormbootx 0.29.0) show only the per-port line (link up, so no #17 lines); no SOL capture has a
  verbose boot. Left: a **verbose** X9 rustnic boot (`nic_verbose = true`, stormbootx#80/#102), asked of the master
  with `wait-hardware` (server1); record HW-checks 17–18 in spec §7.1, then close #17.
- [ ] #16 quiet console by default, trace behind a verbose switch. **Built in v0.2.5 (`04e7d2c`, 2026-10-06)**, ixgbe#22's
  shape: `src/console.rs` (`say!`/`trace!`/`alarm!`/`note!`), `src/trace.rs` (16-line replay ring, `test/trace.rs`),
  `verbose` feature, `StormnicVerbose` (GUID ce1479a2-…-c909ea5b8e0b). sc-build passes (both images, host tests
  9 + 8 + 3). Pinned by stormbootx#104. 2026-11-07: 14 X9 boots at v0.2.5 (server1/2/8) were quiet but printed a
  second line, `port 1: link up`, from the bring-up link wait; **v0.2.7 (`e961b52`)** makes that trace only (sc-build:
  both images, subsystem 11, host tests, repro-check). Pinned v0.2.8 by stormbootx#195 (stormbootx v0.41.0, `cae61d3`, rustnic golden
  `mlx4@f3d3bc2`). 2026-11-10: no X9 SOL capture at that pin yet (server1/2/8 last booted `mlx4@04e7d2c`; only
  pvetest1/2, no ConnectX-3). Asked the master with `wait-hardware` (server1, v0.41.0 rustnic). Left: that boot
  showing exactly the one line per port; then close #16.
- [ ] #31 the stormnic port contract (owner, 2026-11-14): the driver brings its port up itself with the IEEE
  standard FEC for its media/speed and prints one line per port (driver, PCI address, MAC, media, speed, FEC,
  link). ConnectX-3 runs 1G/10G/40G (and 20G/56G, no IEEE FEC): the standard is **none** at every speed it runs,
  and the spec defines no FEC command, so nothing is written. In progress: the per-port line gets `media`
  (module EEPROM, 5.11.5), the rate and `fec none (standard for …)`; a port without link is an `ERROR` line.
- [x] #7 identify/Start error paths logged (v0.2.8, `ae7b5d0`, 2026-11-07): `GetLocation` failure printed, NIC driven
  with location `?`; failed PCI ID read in `Start` printed, its status returned. sc-build: both images, no rustc or
  clippy warnings, host tests. A failed ID read in `Supported` stays silent (not known to be a ConnectX-3).
