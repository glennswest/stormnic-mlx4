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
sc-build 'cargo build --release --target x86_64-unknown-uefi && scripts/pe-subsystem.sh "${CARGO_TARGET_DIR:-target}"/x86_64-unknown-uefi/release/stormnic-mlx4.efi'
```

The second half checks the image is a boot-service driver (PE subsystem 11).
Each sc-build job gets its own drive, deleted afterwards, so the image does not
survive the job; `${CARGO_TARGET_DIR:-target}` finds it wherever the job puts
the target dir.

## How it ships

No standalone component golden. The owner approved shipping inside stormbootx's
`nic-drivers` golden from a pinned `STORMNIC_MLX4_REF` (#8; stormbootx#29).
The mlx4 integration is tracked in stormbootx#34 (#29 closed with ixgbe only).
Until it lands, `--drivers DIR` is the manual
media assembly interface, not a way to retain an sc-build artifact. sc-build
keeps no image. The media/golden work belongs to stormbootx; do not create a
persistent build checkout or copy artifacts out of sc-build.

No configuration, ports or APIs; the only interface is the driver binding
(README, "Interfaces and configuration").

## Test

Put the built `.efi` in a stormbootx ISO's `\stormboot\drivers`, **without**
`ipxe-hermon.efi` (`scripts/build-boot-agent.sh --iso --drivers DIR` in
stormbootx). Boot server1 from the virtual CD, which the stormcentral minismbd
serves as `\boot\stormbootx.iso`. Ask the master to swap the ISO and boot
the blade; the master holds the BMC and console access.

## Version

`Cargo.toml` → `package.version`. Current: `v0.1.0`.

## Work plan

### Issue #1 media dependency recheck (2026-09-28)

- [x] Re-read #1, open issues and the current media implementation.
  stormbootx#29 is closed, but its build script only integrates ixgbe;
  it has no `STORMNIC_MLX4_REF` or mlx4 build. The available server1 SOL
  contains ixgbe binding evidence, but no stormnic-mlx4 lines.
- [x] File the missing mlx4 integration as stormbootx#34.
- [x] Move #1 behind stormbootx#34; stormcentral confirmed the item moved
  back in line (retaining the historical #29 dependency).
- [x] Correct shipping references, push, and verify with sc-build at
  a4c8194: release UEFI build and PE subsystem 11 check passed, remote exit 0.
  The drive was deleted. A local read-only telemetry warning followed success.
  No runtime change or version bump. Keep #1 open until its hardware
  acceptance evidence exists; this needs no new owner decision.

### Issue #1 verification follow-up (2026-09-28)

- [x] Read #1 and review open issues. #8 records the owner's decision to build
  now; #2–#4 remain subsequent implementation work, and #7 tracks logging gaps.
- [x] Update shipping docs to the pinned-driver media path approved in #8 and
  stormbootx#29; this repository still has no standalone golden.
- [x] Push, then rerun the release UEFI build and PE subsystem check with sc-build.
  Passed at f077b04 on 2026-09-28: release x86_64-unknown-uefi build and
  scripts/pe-subsystem.sh reported subsystem 11 (remote exit 0).
- [x] Record verification and move #1 behind stormbootx#29 for hardware acceptance.
  stormcentral confirmed #1 moved back in line after stormbootx#29.
  Do not close #1 until the required Supported/Start console evidence exists.

- [ ] Driver scaffold: `EFI_DRIVER_BINDING_PROTOCOL` matching 15b3:1003/1007, built as an EFI boot-service driver
  - In progress (#1): `build.rs` links `/subsystem:efi_boot_service_driver`; entry installs the binding via `uefi::driver::install`;
    `Supported` reads vendor/device through our own `EFI_PCI_IO_PROTOCOL` binding (uefi-raw has none) and matches 15b3:1003/1007;
    `Start` logs the bind and, until bring-up exists, releases PCI I/O and returns `UNSUPPORTED` so the NIC is left to any other driver.
    Built on dev 2026-09-28 at 3b1ce49: PE subsystem 11, 23040 bytes. Code side done.
    Remaining (needs the master): stormbootx ISO with this .efi and no ipxe-hermon.efi, boot server1, SOL shows
    `driver binding installed` then `0000:05:00.0 15b3:1003 ConnectX-3:` with `Supported: yes` and `Start: bound`. Close #1 on that.
- [ ] Firmware command interface from the PRM: HCR commands, QUERY_FW, MAP_FA/RUN_FW, QUERY_DEV_CAP, INIT_HCA, ICM mapping
  - **Blocked on the owner (#2), 2026-09-29:** the ConnectX-3 PRM is not public (only the ConnectX-4+ PRM is, and its
    command-queue interface is a different design from ConnectX-3's HCR/mailbox). NVIDIA gives it only under a support contract.
    Asked on #2 (labelled `needs-owner`): (1) get the PRM via NVIDIA support, (2) derive from the BSD option of the dual-licensed
    Linux/FreeBSD mlx4 sources with a BSD notice (recommended; iPXE hermon stays off-limits), or (3) park #2–#4.
    No code written. Do not start #2 until the owner answers; option 2 would also change the "Rules for this crate" above and the README.
- [ ] Ethernet data path: EQ, CQ, one send and one receive QP (raw Ethernet), MAC from QUERY_PORT, port bring-up and link state
- [ ] `EFI_SIMPLE_NETWORK_PROTOCOL` on a child handle with a MAC device path
- [ ] Test on server1's ConnectX-3 port (f4:52:14:84:b7:e0, link up on g16): stormbootx prints `tcp4 : available` with only this driver on the media
- [ ] Retire `ipxe-hermon.efi` from the stormbootx media (stormbootx#27)
