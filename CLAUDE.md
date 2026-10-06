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
- Everything the driver does is logged to the console. The only way to debug
  it on the blades is the SOL capture on stormcentral
  (`/var/lib/stormcentral/console/serverN/sol.log`).

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
carries it with no iPXE. The pin is `cf37f8b` (v0.2.1) as of 2026-10-04;
stormbootx#66 asks for v0.2.3 (`0e50017`). `--drivers DIR` is the manual media assembly
interface, not a way to retain an sc-build artifact. sc-build keeps no image.
The media/golden work belongs to stormbootx; do not create a persistent build
checkout or copy artifacts out of sc-build.

No configuration, ports or APIs; the only interface is the driver binding and
the SNP children (README, "Interfaces and configuration").

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

`Cargo.toml` → `package.version`. Current: `v0.2.3`.

## Work plan

Done (history in git and CHANGELOG.md):

- [x] #1 driver binding, #2 firmware command interface, #3 Ethernet data path, #4 SNP on a child handle per
  Ethernet port. Closed 2026-10-01 on server3 (X9, `golden-stormbootx-rustnic-bde9ae3a566c4d7d`, v0.2.1):
  binding, every command through INIT_HCA, SET_PORT/INIT_PORT, steering, SNP installed, TX of a DHCP discover;
  and server1 link-ok at 10G (DAC on the CRS326 sfp-sfpplus1, fixed 10G). RX and a lease were not seen (#18).
- [x] #9 `Cargo.lock` committed, builds `--locked`.
- [x] #10 `docs/spec/connectx3.md` (the only source; BSD notice in `NOTICE`).
- [x] #18 RX, a DHCP lease and an NVMe/TCP claim through the ConnectX-3 (server3 and server8 rustnic boots, v0.2.1,
  2026-10-02..05). Spec §7.1 records HW-checks 1–13; item 14 split to #21 (v0.2.3).
- [x] #15 UAR `BarIndex` from `GetBarAttributes` (v0.2.1), VPI ports forced to Ethernet, link/module
  diagnostics at link wait and link changes (v0.2.2). v0.2.2 pin requested in stormbootx#66.

Open:

- [ ] #21 §7 item 14 (own-frame loopback): v0.2.3 logs the first looped-back own frame; answered by the first
  rustnic boot at a pin with v0.2.3 (asked in stormbootx#66, `0e50017`). Record the result in spec §7.1.
- [ ] #12 §7 items 15 (OS hand-off, `mlx4_core` probe after ExitBootServices) and 16 (`memory region: MPT …, L_Key …`,
  already on the server3 console).
- [ ] #20 (in progress 2026-10-06) spec addition by an independent agent (not the driver author), from the BSD
  mlx4 sources at new pinned commits: module EEPROM read, ACCESS_REG/PTYS speed masks, forced speed/autoneg,
  new §7 HW-checks. The driver author reviews only the spec text. Then #17 (code) can start.
- [ ] #17 DAC link diagnostics beyond spec 3.5: waits on #20.
- [ ] #16 quiet console by default, trace behind a verbose switch.
- [ ] #7 unlogged identify/Start error paths. #13 byte-reproducible image.
- [ ] #5 retire `ipxe-hermon.efi` from the stormbootx media (stormbootx#27; still opt-in there,
  `IPXE_DRIVERS="intelx hermon"`).
