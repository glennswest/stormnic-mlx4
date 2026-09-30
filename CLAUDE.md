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

`Cargo.toml` → `package.version`. Current: `v0.2.0`.

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

### Issue #1 media ready, hardware boot pending (2026-09-29)

- [x] stormbootx#34 closed: rustnic media pins mlx4@cef8dc5 with no iPXE (golden
  `golden-stormbootx-rustnic-b4f33d9566d6127e`); OVMF showed `stormnic-mlx4 0.1.0: driver binding installed`.
- [x] server1 SOL checked 2026-09-29: only the older ixgbe-only run, no stormnic-mlx4 lines. The rustnic boot has not run.
- [x] Asked the owner/master on #1 to boot server1 from the rustnic golden (one boot also checks #2 and #3);
  #1 waits in Needs you. Close #1 when the SOL shows `driver binding installed`, `0000:05:00.0 15b3:1003`,
  `Supported: yes` and `Start: bound; bringing up the firmware` (the old "no firmware bring-up yet" wording is gone).
- [x] Rechecked 2026-09-29: server1 SOL (last written 21:34) still has only ixgbe runs, no stormnic-mlx4 lines.
  Current media is `golden-stormbootx-rustnic-ab4e848a4dcfcaf3` (mlx4 v0.2.0, stormbootx#50); one boot on it checks
  #1–#4. Pass for #1 accepts `stormnic-mlx4 0.1.0` or `0.2.0: driver binding installed`. Question re-posted on #1, needs-owner.

### Issue #9 commit Cargo.lock (2026-09-29)

- [x] Generate `Cargo.lock` on dev (no cargo on this VM): `sc-build 'cargo generate-lockfile && cat Cargo.lock'`,
  resolving `uefi` 0.39.x / `uefi-raw` 0.15.x from the existing constraints. Commit it (not in `.gitignore`).
  Done at 89798b1: uefi 0.39.0, uefi-raw 0.15.1, 17 packages (0.41 is available but outside `0.39`).
- [x] Push, then verify with `sc-build` using `--locked` (release UEFI build + PE subsystem 11 check).
  Passed at b89d437: locked release build, subsystem 11, 23040 bytes, `Cargo.lock` unchanged, exit 0.
- [x] Document `--locked` in README/CLAUDE.md build commands; tell stormbootx#34 it can build locked; close #9.

- [ ] Driver scaffold: `EFI_DRIVER_BINDING_PROTOCOL` matching 15b3:1003/1007, built as an EFI boot-service driver
  - In progress (#1): `build.rs` links `/subsystem:efi_boot_service_driver`; entry installs the binding via `uefi::driver::install`;
    `Supported` reads vendor/device through our own `EFI_PCI_IO_PROTOCOL` binding (uefi-raw has none) and matches 15b3:1003/1007;
    `Start` logs the bind and, until bring-up exists, releases PCI I/O and returns `UNSUPPORTED` so the NIC is left to any other driver.
    Built on dev 2026-09-28 at 3b1ce49: PE subsystem 11, 23040 bytes. Code side done.
    Remaining (needs the master): stormbootx ISO with this .efi and no ipxe-hermon.efi, boot server1, SOL shows
    `driver binding installed` then `0000:05:00.0 15b3:1003 ConnectX-3:` with `Supported: yes` and `Start: bound`. Close #1 on that.
- [ ] Firmware command interface (from the spec): HCR commands, QUERY_FW, MAP_FA/RUN_FW, QUERY_DEV_CAP, INIT_HCA, ICM mapping
  - Owner decided 2026-09-29: implement from `docs/spec/connectx3.md` (merged, #10), BSD notice in `NOTICE`.
  - Plan (#2, in progress): `src/pci.rs` gains BAR MMIO, config writes, AllocateBuffer/Map/Unmap/FreeBuffer, Attributes,
    GetBarAttributes; `src/dma.rs` DMA common buffers + page-list splitting (spec 3.2); `src/hcr.rs` HCR protocol (2.x);
    `src/fw.rs` ownership (1.5), reset (1.6), QUERY_FW, MAP_FA/RUN_FW, MOD_STAT_CFG, QUERY_DEV_CAP, QUERY_PORT,
    QUERY_ADAPTER, profile (3.7), SET_ICM_SIZE/MAP_ICM_AUX/MAP_ICM, INIT_HCA, QUERY_FUNC, and the teardown (3.13).
    `Start` runs bring-up to INIT_HCA, logs it all (section 7 items), tears it down (CLOSE_HCA, UNMAP_*, release
    ownership, restore PCI attributes) and still returns `UNSUPPORTED` until #3/#4 exist. On any failure: reset, never free
    memory the device may still own. Hardware acceptance: server1 SOL on stormbootx-rustnic media (stormbootx#45).
  - Code done at b532094; sc-build 2026-09-29: locked release build, PE subsystem 11, 47616 bytes; clippy clean apart
    from the old `inspect_err` note in main.rs. Remaining: hardware run on server1 (the master swaps in rustnic media with
    this driver pinned, boots the blade). Pass = SOL shows `INIT_HCA (0x0): ok` … `firmware stopped, memory returned` and
    `Start: firmware check passed`. That run also answers spec section 7 items 1–4 (toggle, ownership/semaphore, revision,
    small profile) and 12 (port type); record them on #2 and in the spec's checklist before starting #3.
  - 2026-09-29: status posted on #2, stormbootx#34 asked to pin 69efddd+; #2 moved behind stormbootx#34 (stormcentral confirmed).
    #2 stays open until the server1 SOL shows the pass lines.
- [ ] Ethernet data path: EQ, CQ, one send and one receive QP (raw Ethernet), MAC from QUERY_PORT, port bring-up and link state
  - Plan (#3, 2026-09-29, from spec sections 4–6): `src/dma.rs` gains a copyable `Mem` view (DmaBuf derefs to it);
    `src/eth.rs` creates, in Linux's order, EQ (256 × 32 B) + MAP_EQ, CONF_SPECIAL_QP, the physical MPT (L_Key), then per
    Ethernet port: RX/TX CQs, RX QP (256 × 2 KiB buffers) and TX QP (128 TXBBs), MTT entries written straight into ICM,
    SET_PORT MAC_TABLE/GENERAL (+RQP_CALC in A0), INIT_PORT, B0 MCG attach (port MAC unicast, broadcast), SET_MCAST_FLTR
    DISABLE. Every created object pushes its undo command; teardown pops them LIFO before CLOSE_HCA (spec 6.4 order), and
    all queue memory is freed only after UNMAP_FA or a reset. Self-test per port: wait ≤10 s for link, broadcast a
    DHCPDISCOVER (and an ARP probe for an address learned from the wire), log every frame for 6 s; pass line
    `port N: broadcast round trip ok`. Ports are handled one after the other (object numbers are per port, so #4 can keep
    both up). Still returns `UNSUPPORTED` (no SNP until #4). Hardware acceptance: server1 SOL, same media as #2.
  - Code done 2026-09-29 (bd7b8f9 + fixes); sc-build: locked release build, PE subsystem 11, 71680 bytes, clippy clean
    apart from the old `inspect_err` note. Remaining: hardware run on server1 (same rustnic media as #2, pinned to this
    commit or later). Pass = SOL shows `port N: broadcast round trip ok: ...` for the cabled port, then `firmware stopped,
    memory returned`. The same log answers spec section 7 items 5–11, 13 and 14; record them on #3 and in the spec.
    Close #3 on that; #3 waits on stormbootx#34 like #2.
- [ ] `EFI_SIMPLE_NETWORK_PROTOCOL` on a child handle with a MAC device path
  - Plan (#4, 2026-09-29, spec 6.1 step 28, 6.2–6.4, UEFI spec SNP): `Start` keeps the device: `fw::open` (bring-up,
    returns the `Hca`), `eth::open` (every Ethernet port up to steering, no round-trip check any more), a wait of at
    most 5 s for link, then one child handle per Ethernet port with a MAC device path (parent path + MAC node,
    IfType 1) and an SNP, the parent PCI I/O opened BY_CHILD_CONTROLLER. The driver binding is our own
    (uefi-rs's refuses `Stop` with children). SNP: Start/Stop/Initialize/Shutdown state machine, Transmit copies
    into the bounce buffer and GetStatus returns the caller's buffer once its CQE is reaped, Receive leaves a frame
    queued on BUFFER_TOO_SMALL, receive filters unicast/broadcast/multicast (B0: multicast MACs attached to the RX
    QP, never detached; software filter on top; own-source frames dropped, 5.10), MCastIpToMac, WaitForPacket,
    MediaPresent from port-change events (QUERY_PORT fallback when MAP_EQ failed). Every SNP call runs at
    TPL_CALLBACK. `Stop` uninstalls the children, then tears down (6.4) and frees; ExitBootServices runs 6.4's
    command teardown silently, without freeing, releases ownership and clears bus master (bus master only, if a
    command fails). Hardware acceptance: stormbootx prints `tcp4 : available` on server1 with rustnic media pinned
    to this commit or later.
  - Code done 2026-09-29 (778bc55, ffea25f); sc-build: locked release build with no warnings, PE subsystem 11, clippy
    clean (one `vec_box` allowed: the boxes keep the SNP addresses fixed). Released as v0.2.0. Remaining: stormbootx
    pins `STORMNIC_MLX4_REF` to v0.2.0 on the rustnic media (stormbootx#50), the master boots
    server1; pass = `tcp4 : available` and `port 1 SNP: initialized`. Close #4 on that.
- [ ] Test on server1's ConnectX-3 port (f4:52:14:84:b7:e0, link up on g16): stormbootx prints `tcp4 : available` with only this driver on the media
- [ ] Retire `ipxe-hermon.efi` from the stormbootx media (stormbootx#27)
