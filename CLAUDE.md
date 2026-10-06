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
The second half checks the image is a boot-service driver (PE subsystem 11).
Each sc-build job gets its own drive, deleted afterwards, so the image does not
survive the job; `${CARGO_TARGET_DIR:-target}` finds it wherever the job puts
the target dir.

## How it ships

No standalone component golden. The owner approved shipping inside stormbootx's
`nic-drivers` build from a pinned `STORMNIC_MLX4_REF` (#8). stormbootx#34 did
the mlx4 integration: `scripts/build-nic-drivers.sh` builds this crate at the
pin, and the `stormbootx-rustnic` media (`STORMNIC_ON_MEDIA="ixgbe mlx4"`)
carries it with no iPXE. The pin was `cf37f8b` (v0.2.1) as of 2026-10-04;
stormbootx#66 pinned v0.2.3 (`0e50017`); stormbootx#104 asks for v0.2.4 (`32083cd`). `--drivers DIR` is the manual media assembly
interface, not a way to retain an sc-build artifact. sc-build keeps no image.
The media/golden work belongs to stormbootx; do not create a persistent build
checkout or copy artifacts out of sc-build.

No ports or APIs; the interfaces are the driver binding, the SNP children, and one console switch, the EFI
variable `StormnicVerbose` (#16; README, "Interfaces and configuration").

## Test

Host tests: `sc-build 'scripts/test-host.sh'` (`test/bars.rs`).

Hardware: put the built `.efi` in a stormbootx ISO's `\stormboot\drivers`,
**without** `ipxe-hermon.efi` (`scripts/build-boot-agent.sh --iso --drivers DIR`
in stormbootx), or use the rustnic golden at the current pin. Boot a blade from
the virtual CD, which the stormcentral minismbd serves as `\boot\stormbootx.iso`.
**The blade boot is the master's job, not an owner decision** (master on #1,
2026-09-30): ask the master on the issue, never with `needs-owner`;
stormcentral#237 is to settle a proper hardware-boot request. stormbootx does
not use the firmware's TCP4: its console line is `tcp4 : smoltcp over SNP
(<nics>)`, never `tcp4 : available`.

## Version

`Cargo.toml` → `package.version`. Current: `v0.2.5`.

## Work plan

Done (history in git and CHANGELOG.md):

- [x] #1 driver binding, #2 firmware command interface, #3 Ethernet data path, #4 SNP on a child handle per
  Ethernet port. Closed 2026-10-01 on server3 (X9, `golden-stormbootx-rustnic-bde9ae3a566c4d7d`, v0.2.1):
  binding, every command through INIT_HCA, SET_PORT/INIT_PORT, steering, SNP installed, TX of a DHCP discover;
  and server1 link-ok at 10G (DAC on the CRS326 sfp-sfpplus1, fixed 10G). RX and a lease were not seen (#18).
- [x] #9 `Cargo.lock` committed, builds `--locked`.
- [x] #10 `docs/spec/connectx3.md` (the only source; BSD notice in `NOTICE`). #20 added 5.11–5.13 (EEPROM, PTYS, speed).
- [x] #18 RX, a DHCP lease and an NVMe/TCP claim through the ConnectX-3 (server3 and server8 rustnic boots, v0.2.1,
  2026-10-02..05). Spec §7.1 records HW-checks 1–13; item 14 split to #21 (v0.2.3).
- [x] #15 UAR `BarIndex` from `GetBarAttributes` (v0.2.1), VPI ports forced to Ethernet, link/module
  diagnostics at link wait and link changes (v0.2.2). v0.2.2 pin requested in stormbootx#66.

Open:

- [ ] #21 §7 item 14 (own-frame loopback): v0.2.3 logs the first looped-back own frame; answered by the first
  rustnic boot at a pin with v0.2.3 (asked in stormbootx#66, `0e50017`). Record the result in spec §7.1.
- [ ] #12 §7 items 15 (OS hand-off, `mlx4_core` probe after ExitBootServices) and 16 (`memory region: MPT …, L_Key …`,
  already on the server3 console).
- [ ] #17 DAC link diagnostics, now specified (#20 done 2026-10-06: spec 5.11–5.13, HW-checks 17–19). Item 1
  (module EEPROM via MAD_IFC 0xFF60, spec 5.11) can be built. Items 2–3 need PTYS, gated on QUERY_DEV_CAP 0x7a
  bit 5, which the blades' firmware 2.30.8000 leaves 0 (spec 5.13): log that, no speed setting (stormbootx#80).
  **Built in v0.2.4 (`32083cd`, 2026-10-06):** `src/diag.rs`/`src/module.rs` print the module EEPROM (5.11) and a
  read-only PTYS query (5.12) after the link wait and at every link-down, plus a `speed control:` line; no PTYS
  write (item 3 is not offered on the blades, 5.13). sc-build passes (subsystem 11, host tests 9 + 8). Pin asked in
  stormbootx#104. Left: a rustnic boot at that pin; record HW-checks 17–18 in spec §7.1, then close #17.
- [ ] #16 quiet console by default, trace behind a verbose switch. **In progress (2026-10-06):** ixgbe#22's shape
  (owner on #16): `src/console.rs` (`say!` always, `trace!` verbose or kept in a 16-line ring, `fail!` replays the
  ring then prints), `src/trace.rs` (the ring, host test `test/trace.rs`), `verbose` feature, EFI variable
  `StormnicVerbose` (GUID ce1479a2-eab9-4176-b0ad-c909ea5b8e0b, first byte non-zero) read once at the entry point.
  Default: one line per port (`stormnic-mlx4 X.Y.Z: LOC 15b3:DDDD NAME: port N MAC …, link …, SNP installed`) plus
  warnings/errors; the command trace, ICM sizes, per-frame lines and the #17 diagnostics on a good link go to trace.
- [ ] #7 unlogged identify/Start error paths. #13 byte-reproducible image.
- [ ] #5 retire `ipxe-hermon.efi` from the stormbootx media (stormbootx#27; still opt-in there,
  `IPXE_DRIVERS="intelx hermon"`).
