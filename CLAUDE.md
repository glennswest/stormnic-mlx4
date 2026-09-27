# CLAUDE.md — stormnic-mlx4

A `no_std` UEFI driver giving firmware an `EFI_SIMPLE_NETWORK_PROTOCOL` for
Mellanox ConnectX-3 / ConnectX-3 Pro Ethernet (the Linux `mlx4` family). It's loaded by stormbootx from `\stormboot\drivers` on its boot
media, on machines whose firmware has the TCP/IP stack but no UEFI driver for
the NIC (the Supermicro X9 blades, stormbootx#26). Read README.md first.

Read the cross-project rules in `../CLAUDE.md` first. In particular, **build
with `sc-build` after pushing, never on this VM and never as root**, and
scratch files go in `tmp/`.

## Rules for this crate

- **Written from the vendor documentation** (Mellanox ConnectX-3 Programmer's Reference Manual (PRM); the firmware command interface (HCR, mailboxes, EQ/CQ/QP) is the bulk of the work), **not translated
  from iPXE or Linux.** The crate is MIT; a port of GPL code would not be.
  Reading them for behaviour is fine.
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
sc-build 'cargo build --release --target x86_64-unknown-uefi'
```

## Test

Put the built `.efi` in a stormbootx ISO's `\stormboot\drivers`, **without**
`ipxe-hermon.efi` (`scripts/build-boot-agent.sh --iso --drivers DIR` in
stormbootx). Boot server1 from the virtual CD, which the stormcentral minismbd
serves as `\boot\stormbootx.iso`. Ask the master to swap the ISO and boot
the blade; the master holds the BMC and console access.

## Version

`Cargo.toml` → `package.version`. Current: `v0.1.0`.

## Work plan

- [ ] Driver scaffold: `EFI_DRIVER_BINDING_PROTOCOL` matching 15b3:1003/1007, built as an EFI boot-service driver
- [ ] Firmware command interface from the PRM: HCR commands, QUERY_FW, MAP_FA/RUN_FW, QUERY_DEV_CAP, INIT_HCA, ICM mapping
- [ ] Ethernet data path: EQ, CQ, one send and one receive QP (raw Ethernet), MAC from QUERY_PORT, port bring-up and link state
- [ ] `EFI_SIMPLE_NETWORK_PROTOCOL` on a child handle with a MAC device path
- [ ] Test on server1's ConnectX-3 port (f4:52:14:84:b7:e0, link up on g16): stormbootx prints `tcp4 : available` with only this driver on the media
- [ ] Retire `ipxe-hermon.efi` from the stormbootx media (stormbootx#27)
