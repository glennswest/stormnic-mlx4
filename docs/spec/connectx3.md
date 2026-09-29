# ConnectX-3 / ConnectX-3 Pro programming specification (Ethernet, UEFI polling driver)

Status: first edition, 2026-09-29 (issue #10). Written by an agent that does not
write the driver. The driver (#2, #3, #4) is implemented from this document; its
author should not need to read any other driver.

## 0. About this document

### 0.1 Scope

This document describes how system software drives a Mellanox/NVIDIA
**ConnectX-3 (PCI 15b3:1003)** or **ConnectX-3 Pro (PCI 15b3:1007)** adapter as
a single Ethernet port, with polling only (no interrupts), from PCI probe to
sending and receiving frames and back to a quiescent device. It covers:

1. PCI/BAR layout, device reset, ownership, and the Host Command Register (HCR)
   firmware command protocol (section 1 and 2).
2. Firmware bring-up: QUERY_FW, MAP_FA/RUN_FW, MOD_STAT_CFG, QUERY_DEV_CAP,
   QUERY_PORT, QUERY_ADAPTER, the ICM (context memory) profile, SET_ICM_SIZE,
   MAP_ICM_AUX, MAP_ICM, INIT_HCA and the teardown commands (section 3).
3. The context tables and objects one Ethernet port needs: MTT, MPT (memory
   region), EQ, CQ, QP, UAR doorbells (section 4).
4. The Ethernet data path: port configuration (SET_PORT, INIT_PORT), receive
   steering, send and receive work queue entries, completion queue entries,
   doorbells, link state, MAC address, MTU (section 5).
5. A step-by-step minimal polling driver and its shutdown on
   ExitBootServices (section 6).
6. Items that must be confirmed on hardware (section 7), and appendices of
   constants, opcodes and a glossary.

Out of scope: InfiniBand, RDMA, SR-IOV/virtual functions (15b3:1004), the
multi-function "communication channel", BlueFlame, RSS, checksum and
segmentation offloads, device-managed flow steering, interrupts (MSI-X and
INTx), timestamping, and firmware flashing.

### 0.2 Provenance and licence

Everything here was derived by reading the Linux `mlx4_core`/`mlx4_en` drivers
and the FreeBSD `mlx4` drivers. Every file used is offered under a choice of
GPL-2 **or the OpenIB.org BSD licence**; this document uses the BSD option.
`NOTICE` at the repository root carries that licence text. No code was copied:
the document states behaviour, layouts and constants in prose and tables.
Nothing was taken from iPXE (hermon) or any GPL-only source.

Baselines read:

| Tree | Commit | Paths |
|---|---|---|
| Linux (torvalds/linux) | `6f8319e3e9a44dd537d17f41565a8453c560a581` (2026-09-28) | `drivers/net/ethernet/mellanox/mlx4/`, `include/linux/mlx4/` |
| FreeBSD (freebsd/freebsd-src) | `669cd0d90ee7b5bed794ded5fa00b6be854a6598` (2026-09-29) | `sys/dev/mlx4/` |

Each section ends with a **Sources** line naming the Linux file and function(s)
a reviewer should compare against. FreeBSD carries the same functions under a
different file naming scheme; the correspondence is:

| Linux (`drivers/net/ethernet/mellanox/mlx4/`) | FreeBSD (`sys/dev/mlx4/`) |
|---|---|
| `cmd.c`, `fw.c`, `main.c`, `icm.c`, `profile.c`, `eq.c`, `cq.c`, `qp.c`, `mr.c`, `pd.c`, `port.c`, `mcg.c`, `reset.c`, `catas.c`, `alloc.c`, `sense.c` | `mlx4_core/mlx4_<same name>.c` |
| `mlx4.h`, `fw.h`, `icm.h` | `mlx4_core/mlx4.h`, `mlx4_core/fw.h`, `mlx4_core/icm.h` |
| `en_netdev.c`, `en_rx.c`, `en_tx.c`, `en_cq.c`, `en_resources.c`, `en_port.c`, `en_main.c`, `mlx4_en.h`, `en_port.h` | `mlx4_en/mlx4_en_<same>.c`, `mlx4_en/en.h`, `mlx4_en/en_port.h` |
| `include/linux/mlx4/{cmd,device,qp,cq,doorbell}.h` | `sys/dev/mlx4/{cmd,device,qp,cq,doorbell}.h` |

One layout rule (send/receive queue order inside a QP buffer, 4.8.2) is stated
more explicitly in the InfiniBand-side driver, FreeBSD
`mlx4_ib/mlx4_ib_qp.c` (`set_kernel_sq_size`), which is under the same dual
licence.

### 0.3 Conventions

- **Byte order.** Everything the device reads or writes is **big-endian**:
  HCR registers, mailboxes, context entries, work queue entries (WQEs),
  completion and event queue entries (CQEs, EQEs), doorbell records in host
  memory and doorbell writes to MMIO. On an x86 host every multi-byte field
  must be byte-swapped. The notation `be16`, `be32`, `be64` marks a big-endian
  field of that width; `u8` is a single byte.
- **Offsets** are byte offsets from the start of the structure, in hex.
- **Bits** are numbered within the big-endian value they belong to, bit 0 being
  the least significant. "Dword at 0x14, bit 31" is the most significant bit of
  byte 0x14. When a field is shown as a byte, its bits are numbered 7..0 within
  that byte.
- **Addresses.** "Physical address" or "DMA address" always means the address
  the device uses on the bus, as returned by `EFI_PCI_IO_PROTOCOL.Map` (for
  UEFI) for a common buffer. The device does 64-bit DMA; enable
  `EFI_PCI_IO_ATTRIBUTE_DUAL_ADDRESS_CYCLE`, or allocate everything below
  4 GiB.
- **Pages.** The device's page ("ICM page") is 4 KiB. log2 values called
  `log_page_size` in contexts are expressed as "log2(page bytes) − 12".
- `roundup_pow2(x)` is the smallest power of two ≥ x; `log2` of a power of two
  is exact.
- **MUST / SHOULD** indicate what Linux always does versus what it does only
  in some configurations. "Recommended" marks a choice this document makes for
  a minimal driver where Linux offers several.
- Items marked **[HW-CHECK]** are inferences that must be confirmed on a card
  (collected in section 7).

---

## 1. PCI function, BARs, ownership and reset

### 1.1 Identification

| Device | Vendor | Device ID | Linux table entry |
|---|---|---|---|
| ConnectX-3 (MT27500 family) | 0x15b3 | 0x1003 | physical function, no forced port sensing |
| ConnectX-3 Pro (MT27520 family) | 0x15b3 | 0x1007 | same |
| ConnectX-3 virtual function | 0x15b3 | 0x1004 | **not supported** (needs the multi-function channel) |

The same `mlx4` protocol also covers 0x1005–0x1010 (other MT27xxx family
members); this document does not claim they behave identically.

A card may be single- or dual-port, and each port may be Ethernet-only or VPI
(InfiniBand/Ethernet). Section 5.1 covers port type.

Sources: `main.c` — `mlx4_pci_table`.

### 1.2 BARs

| Linux resource | Config offset | Type | Contents |
|---|---|---|---|
| BAR 0 | 0x10 (64-bit) | memory | Device configuration space ("DCS"): HCR, ownership register, reset block, catastrophic-error buffer, and other firmware-described areas. Must be present, else the device is unusable. |
| BAR 2 | 0x18 (64-bit) | memory | UAR (User Access Region) pages: doorbells. The first `2^n` MiB (n from QUERY_DEV_CAP 0x49) is the UAR area; the rest is BlueFlame (unused here). Must be present. |

Both BARs are 64-bit, so they occupy config registers 0x10/0x14 and 0x18/0x1c.
In EDK2's `EFI_PCI_IO_PROTOCOL`, `BarIndex` is the BAR register index, so the
DCS is `BarIndex` 0 and the UAR area is `BarIndex` 2.

Where the firmware reports a BAR in a command output (QUERY_FW), it gives a
2-bit value in bits 7:6 of a byte; the BAR register index is that value × 2
(so 0 → BAR 0, 1 → BAR 2).

Sources: `main.c` — `__mlx4_init_one` (BAR checks), `mlx4_dev_cap` (UAR size check); `fw.c` — `mlx4_QUERY_FW` (BAR decoding).

### 1.3 BAR 0 register map (fixed offsets)

| Offset in BAR 0 | Size | Name | Access |
|---|---|---|---|
| 0x80680 | 0x1c (7 dwords) | HCR — Host Command Register | 32-bit reads/writes only, big-endian (section 2) |
| 0x8069c | 4 | Ownership semaphore | read to claim, write 0 to release (1.5) |
| 0xf0000 | 0x400 | Reset block | see 1.6 |
| 0xf0010 | 4 | Reset register (in reset block) | write big-endian 0x00000001 |
| 0xf03fc | 4 | Hardware semaphore (in reset block) | read until 0 (1.6) |
| from QUERY_FW | QUERY_FW | Catastrophic error buffer | read-only; first dword non-zero means firmware fatal error (2.8) |

**Writes of less than 32 bits to the HCR do not work.** Always use aligned
32-bit accesses in BAR 0.

Sources: `mlx4.h` — `MLX4_HCR_BASE`, `MLX4_HCR_SIZE`; `main.c` — `MLX4_OWNER_BASE`, `mlx4_get_ownership`; `reset.c` — `mlx4_reset`; `cmd.c` — `mlx4_cmd_post` (comment on sub-32-bit writes).

### 1.4 BAR 2: UAR pages

The UAR area is an array of 4 KiB pages. Page `i` starts at BAR 2 + i × 4096.
A page is identified by its **UAR index** `i`; QP and CQ contexts name the UAR
page whose doorbells they use (`usr_page`, `logsize_usrpage`).

Doorbell locations inside a UAR page:

| Offset in page | Width | Use |
|---|---|---|
| 0x014 | 32-bit | Send doorbell ("send queue has new work"), 4.8.6 / 5.7 |
| 0x020 | 64-bit | CQ arm doorbell (request an event); unused by a polling driver |
| 0x800 + 8 × (eqn mod 4) | 32-bit | EQ consumer-index doorbell for EQ number `eqn`, which lives in UAR page `eqn / 4` (integer division) |

Reserved pages: the first 128 UAR pages (with a 4 KiB host page) are reserved
for EQ doorbells; more may be reserved by firmware (QUERY_DEV_CAP 0x48). A
driver MUST choose its own UAR page index ≥ max(128, reserved UARs); section
4.1 gives the rule. There must be more than 128 UAR pages; if the UAR area has
128 pages or fewer, firmware configuration is unusable (Linux fails with
"Increase firmware log2_uar_bar_megabytes").

Sources: `include/linux/mlx4/doorbell.h` — `MLX4_SEND_DOORBELL`, `MLX4_CQ_DOORBELL`; `eq.c` — `mlx4_get_eq_uar`; `include/linux/mlx4/device.h` — `mlx4_get_num_reserved_uar`, `mlx4_to_hw_uar_index`; `pd.c` — `mlx4_init_uar_table`, `mlx4_uar_alloc`.

### 1.5 Ownership semaphore

Before touching the device, a physical-function driver claims it:

1. Read the dword at BAR 0 + 0x8069c.
2. If it reads **0**, this driver now owns the device. If it reads non-zero,
   another function (or driver instance) owns it; do not proceed ("multiple
   PFs not supported").

To release (after the device is fully torn down, or on a failed start): write
0 to the same dword, then wait 1000 ms before anyone may claim it again (Linux
sleeps 1 s after the write).

The read has a side effect (it takes the semaphore). Read it exactly once per
claim.

Sources: `main.c` — `mlx4_get_ownership`, `mlx4_free_ownership`, `mlx4_load_one`.

### 1.6 Device reset

Linux resets the device at every load, before any command, "since a boot ROM
may have left the HCA in an undefined state". A UEFI driver MUST do the same:
an option ROM or a previous driver may have left firmware running.

Procedure:

1. Save PCI configuration space dwords 0..63 (offsets 0x00–0xfc), skipping
   dwords 22 and 23 (offsets 0x58, 0x5c). Also note the PCI Express capability
   position (to restore Device Control and Link Control).
2. Acquire the hardware semaphore: read BAR 0 + 0xf03fc repeatedly (1 ms apart)
   until it reads 0. Give up after 10 s. (The semaphore locks out flash updates
   during reset.)
3. Write the big-endian value 1 (bytes on the bus: 00 00 00 01; on a
   little-endian CPU the 32-bit store value is 0x01000000) to BAR 0 + 0xf0010.
4. Wait **1000 ms** without touching the device.
5. Poll the PCI vendor ID (config offset 0) every 1 ms for up to 2 s until it
   reads other than 0xffff. If it still reads 0xffff, the device did not come
   back.
6. Restore, in this order: PCI Express Device Control (capability + 0x08,
   16-bit) and Link Control (capability + 0x10, 16-bit); then config dwords
   0..15 except the Command register dword (offset 0x04); then the Command
   register dword last.

After reset: firmware is not running (QUERY_FW works, other commands need
MAP_FA/RUN_FW), no context memory is mapped, and the HCR is idle.

Note that the semaphore is not explicitly released; the reset releases it.
**[HW-CHECK]** that reading 0 is indeed "acquired" and that nothing else is
needed.

Sources: `reset.c` — `mlx4_reset`; `main.c` — `mlx4_load_one`.

### 1.7 PCI command register and interrupts

- Set Memory Space Enable (bit 1) and Bus Master Enable (bit 2).
- A polling driver sets Interrupt Disable (bit 10) and never enables MSI or
  MSI-X. Event queues still exist (a CQ must name one) but nothing raises an
  interrupt; see 4.6.
- DMA: 64-bit addresses are supported. Linux allows DMA segments up to 1 GiB.

Sources: `main.c` — `__mlx4_init_one` (`pci_set_master`, DMA mask, max segment).

---

## 2. The firmware command interface (HCR)

All configuration is done by firmware commands posted through the HCR. In
this document commands are always issued in **polling mode** (the "E" bit is
0); only one command is outstanding at a time.

### 2.1 HCR layout

Seven big-endian dwords at BAR 0 + 0x80680:

| Offset | Name | Contents |
|---|---|---|
| 0x00 | in_param high | bits 63:32 of the 64-bit input parameter |
| 0x04 | in_param low | bits 31:0 of the input parameter |
| 0x08 | in_modifier | 32-bit input modifier |
| 0x0c | out_param high | bits 63:32 of the output parameter |
| 0x10 | out_param low | bits 31:0 of the output parameter |
| 0x14 | token | bits 31:16 = token; bits 15:0 = 0 |
| 0x18 | status / go | see 2.2 |

The **input parameter** is either an immediate 64-bit value or the physical
address of an **input mailbox**. The **output parameter** is either the
physical address of an **output mailbox** (written by the driver before
posting) or, for commands with an immediate output, a value the firmware
writes back into these two dwords.

### 2.2 The status/go dword (offset 0x18)

Written by the driver:

| Bits | Field |
|---|---|
| 31:24 | 0 when writing |
| 23 | **GO** — 1 hands the command to firmware |
| 22 | **E** — event: 1 = report completion through an EQ; always 0 here |
| 21 | **T** — toggle (2.4) |
| 19:12 | op_modifier (8 bits) |
| 11:0 | opcode (12 bits) |

Read back:

| Bits | Field |
|---|---|
| 31:24 | command **status** (valid once GO reads 0), table in 2.6 |
| 23 | GO — 1 while firmware owns the HCR |
| 21 | T — see 2.4 |

### 2.3 Posting a command

1. Check the HCR is not pending (2.4). In polling mode Linux does not wait
   here: if the HCR is still pending, posting fails immediately.
2. Write dwords 0x00, 0x04, 0x08, 0x0c, 0x10 and 0x14 (token 0xffff for
   polling: write 0xffff0000), each as one 32-bit big-endian store.
3. Issue a write barrier so those stores reach the device before the next one.
4. Write dword 0x18 = GO (1<<23) | T (toggle<<21) | (op_modifier<<12) | opcode.
5. Flip the driver's `toggle` variable (0↔1).
6. Poll (2.4) until not pending or until the command's timeout (2.7) expires.
7. Read status = byte at 0x18 (bits 31:24). Non-zero is a failure (2.6).
8. For an immediate output, read the 64-bit value from dwords 0x0c (high) and
   0x10 (low).

### 2.4 The toggle bit and the pending test

The driver keeps a one-bit variable `toggle`, set to **1** when the command
interface is initialised (after reset). The HCR is **pending** (busy) when

- GO (bit 23 of the dword at 0x18) reads 1, **or**
- the T bit (bit 21) reads equal to the driver's current `toggle`.

Posting writes T = `toggle` and then flips `toggle`; so a completed command
reads back GO = 0 and T equal to the value just written (≠ the new `toggle`).
After reset, T reads 0, so the first command is posted with T = 1.

Recommended robustness step for a UEFI driver: after reset, read the dword at
0x18; if GO is 0, set `toggle` = NOT(T). This equals Linux's initial value
when T reads 0, and avoids a spurious "pending" if some earlier user left T at
1 and the reset did not clear it. **[HW-CHECK]** the T value after reset.

### 2.5 Mailboxes

- A mailbox is a 4096-byte buffer, **4096-byte aligned**, in DMA-able memory.
  Linux zeroes every mailbox before use; do the same (reserved fields must be
  0).
- Input mailbox: driver fills it, passes its physical address as in_param.
- Output mailbox: driver passes its physical address as out_param; firmware
  fills it. Read it only after the command completes.
- A command may use both (none in this document do, except that QP/MCG
  commands use one input mailbox and READ_MCG uses one output mailbox).
- The device reads/writes the mailbox by DMA, so the buffer must remain mapped
  until the command completes.

Sources: `include/linux/mlx4/cmd.h` — `MLX4_MAILBOX_SIZE`; `cmd.c` — `mlx4_cmd_init` (pool alignment), `mlx4_alloc_cmd_mailbox`.

### 2.6 Status codes

| Status | Meaning |
|---|---|
| 0x00 | OK |
| 0x01 | Internal error (for example a bus error) |
| 0x02 | Bad opcode, or opcode modifier not supported |
| 0x03 | Bad parameter (not supported or out of range) |
| 0x04 | Bad system state (system not enabled) |
| 0x05 | Bad resource (reserved or unallocated resource) |
| 0x06 | Resource busy |
| 0x08 | Exceeds device limits |
| 0x09 | Resource not in the appropriate state or ownership |
| 0x0a | Index out of range |
| 0x0b | Firmware image corrupted (NVMEM) |
| 0x0c | ICM error (e.g. not enough auxiliary ICM pages for the command) |
| 0x10 | QP not in the presumed state |
| 0x20 | Bad segment parameters (address/size) |
| 0x21 | Memory region has memory windows bound |
| 0x22 | HCA local attached memory not present |
| 0x30 | Bad management packet (silently discarded) |
| 0x40 | More outstanding CQEs in the CQ than the new CQ size |
| 0x50 | Multi-function device support required (QUERY_FW returns this on a non-primary function) |

Any other value: treat as an internal error.

Sources: `cmd.c` — status enum, `mlx4_status_to_errno`.

### 2.7 Timeouts

| Item | Value |
|---|---|
| Command completion timeout (all classes A, B, C) | 60 000 ms |
| Wait for GO to clear before posting, event mode only | 10 000 ms (not used) |
| Poll interval | as fast as desired; Linux just yields the CPU between reads |

On timeout (GO still 1), Linux declares the device in internal error; the only
recovery is a device reset (1.6) and full re-initialisation.

Sources: `include/linux/mlx4/cmd.h` — `MLX4_CMD_TIME_CLASS_*`; `cmd.c` — `GO_BIT_TIMEOUT_MSECS`, `mlx4_cmd_poll`.

### 2.8 Fatal errors

- A failure of any of these commands is **fatal** (device must be reset):
  CLOSE_HCA, HW2SW_EQ, HW2SW_CQ, 2RST_QP, HW2SW_SRQ, SYNC_TPT, UNMAP_ICM,
  UNMAP_ICM_AUX, UNMAP_FA, and HW2SW_MPT with any status other than 0x21.
- The **catastrophic error buffer** (location from QUERY_FW: BAR from byte
  0x3c bits 7:6 × 2, offset from be64 at 0x30, size in dwords from be32 at
  0x38) is polled by Linux once a second: if its first dword is non-zero the
  firmware has hit a fatal error and the device must be reset. A polling
  driver MAY check it when a command or the data path stalls.

Sources: `cmd.c` — `mlx4_closing_cmd_fatal_error`; `catas.c` — `poll_catas`, `mlx4_start_catas_poll`; `fw.c` — `mlx4_QUERY_FW`.

### 2.9 Commands used in this document

"imm" = immediate value in in_param/out_param; "in MB"/"out MB" = mailbox.

| Command | Opcode | op_mod | in_param | in_modifier | Output | Section |
|---|---|---|---|---|---|---|
| QUERY_FW | 0x004 | 0 | 0 | 0 | out MB | 3.1 |
| MAP_FA | 0xfff | 0 | in MB | entry count | — | 3.2 |
| RUN_FW | 0xff6 | 0 | 0 | 0 | — | 3.2 |
| MOD_STAT_CFG | 0x034 | 0 | in MB | 0 | — | 3.3 |
| QUERY_DEV_CAP | 0x003 | 0 | 0 | 0 | out MB | 3.4 |
| QUERY_PORT | 0x043 | 0 | 0 | port (1 or 2) | out MB | 3.5 |
| QUERY_ADAPTER | 0x006 | 0 | 0 | 0 | out MB | 3.6 |
| SET_ICM_SIZE | 0xffd | 0 | imm: ICM size in bytes | 0 | imm: aux pages | 3.8 |
| MAP_ICM_AUX | 0xffc | 0 | in MB | entry count | — | 3.8 |
| MAP_ICM | 0xffa | 0 | in MB | entry count | — | 3.9 |
| INIT_HCA | 0x007 | 0 | in MB | 0 | — | 3.10 |
| QUERY_FUNC | 0x056 | 0 | 0 | 0 (function) | out MB | 3.11 |
| CONF_SPECIAL_QP | 0x023 | 0 | 0 | base QPN (0 = disable) | — | 3.12 |
| SW2HW_MPT | 0x00d | 0 | in MB | MPT index | — | 4.3 |
| HW2SW_MPT | 0x00f | 1 | 0 | MPT index | — | 4.3 |
| WRITE_MTT | 0x011 | 0 | in MB | entry count | — | 4.2 (alternative) |
| MAP_EQ | 0x012 | 0 | imm: event mask | (unmap<<31) \| eqn | — | 4.6 |
| SW2HW_EQ | 0x013 | 0 | in MB | eqn | — | 4.6 |
| HW2SW_EQ | 0x014 | 1 | 0 | eqn | — | 4.6 |
| SW2HW_CQ | 0x016 | 0 or 1 | in MB | cqn | — | 4.7 |
| HW2SW_CQ | 0x017 | 1 | 0 | cqn | — | 4.7 |
| RST2INIT_QP | 0x019 | 0 | in MB | qpn | — | 4.8 |
| INIT2RTR_QP | 0x01a | 0 | in MB | qpn | — | 4.8 |
| RTR2RTS_QP | 0x01b | 0 | in MB | qpn | — | 4.8 |
| 2RST_QP | 0x021 | 2 | 0 | qpn | — | 4.8 |
| QUERY_QP | 0x022 | 0 | 0 | qpn | out MB (context at +8) | 4.8 |
| SET_PORT | 0x00c | 1 (Ethernet) | in MB | (sub-op<<8) \| port | — | 5.3 |
| INIT_PORT | 0x009 | 0 | 0 | port | — | 5.5 |
| CLOSE_PORT | 0x00a | 0 | 0 | port | — | 5.5 |
| SENSE_PORT | 0x04d | 0 | 0 | port | imm: port type | 5.1 |
| MGID_HASH | 0x027 | see 5.4 | in MB (16-byte GID) | 0 | imm: hash index | 5.4 |
| READ_MCG | 0x025 | 0 | 0 | MCG index | out MB | 5.4 |
| WRITE_MCG | 0x026 | 0 (entry) / 1 (promisc) | in MB | index, or (port<<16)\|(steer<<1) | — | 5.4 |
| SET_MCAST_FLTR | 0x048 | mode | imm: MAC \| (clear<<63) | port | — | 5.4 |
| CLOSE_HCA | 0x008 | 0 | 0 | 0 | — | 3.13 |
| UNMAP_ICM | 0xff9 | 0 | imm: ICM virtual address | page count (4 KiB) | — | 3.13 |
| UNMAP_ICM_AUX | 0xffb | 0 | 0 | 0 | — | 3.13 |
| UNMAP_FA | 0xffe | 0 | 0 | 0 | — | 3.13 |
| NOP | 0x031 | 0 | 0 | 0x1f | — | optional liveness check |

The full opcode list is in Appendix A.

Sources: `include/linux/mlx4/cmd.h` — opcode enum; `cmd.c` — `mlx4_cmd_post`, `cmd_pending`, `mlx4_cmd_poll`, `__mlx4_cmd`, `mlx4_cmd_init`; the per-command wrappers named in each section.

---

## 3. Firmware bring-up

### 3.1 QUERY_FW

Issue after reset. Output mailbox (0x100 bytes meaningful):

| Offset | Width | Field |
|---|---|---|
| 0x00 | be16 | **FW area size in 4 KiB pages** (pages to give to MAP_FA) |
| 0x02 | be16 | firmware version: major |
| 0x04 | be16 | firmware version: sub-minor |
| 0x06 | be16 | firmware version: minor |
| 0x09 | u8 | PPF ID (this function's number) |
| 0x0a | be16 | **command interface revision** |
| 0x0f | u8 | log2 of the maximum number of outstanding commands |
| 0x20 | be64 | clear-interrupt register offset (INTx only; unused) |
| 0x28 | u8 | bits 7:6: clear-interrupt BAR (× 2 = BAR register index) |
| 0x30 | be64 | catastrophic error buffer offset within its BAR |
| 0x38 | be32 | catastrophic error buffer size, in dwords |
| 0x3c | u8 | bits 7:6: catastrophic error buffer BAR (× 2) |
| 0x40 | be64 | communication channel offset (multi-function only; unused) |
| 0x48 | u8 | bits 7:6: communication channel BAR (× 2) |
| 0x50 | be64 | internal clock offset (unused) |
| 0x58 | u8 | bits 7:6: internal clock BAR (× 2) |

Version printing convention: major.minor.subminor (note sub-minor is stored
before minor).

Checks:

- Status 0x50 (multi-function required): this is not the primary physical
  function; stop.
- Command interface revision MUST be 2 or 3. Revision 2 means "old port
  commands": QUERY_PORT and INIT_PORT behave differently (3.5, 5.5). ConnectX-3
  firmware is expected to report 3 **[HW-CHECK]**.

Sources: `fw.c` — `mlx4_QUERY_FW`, `MLX4_COMMAND_INTERFACE_*`.

### 3.2 Firmware area: MAP_FA and RUN_FW

The firmware runs out of host memory that the driver lends it.

1. Allocate `fw_pages × 4096` bytes of DMA-able memory (QUERY_FW 0x00). It
   need not be one contiguous block, but every block handed to the device must
   be a power of two in size, at least 4 KiB, and **aligned to its own size**.
   The driver never reads or writes this memory.
2. Describe the blocks with MAP_FA (mailbox format below) and issue it.
3. Issue RUN_FW (no parameters).

**Page-list mailbox format** (shared by MAP_FA, MAP_ICM_AUX and MAP_ICM): an
array of 16-byte entries starting at mailbox offset 0; at most 256 entries
(4096/16) per command; in_modifier = number of entries in this command. Issue
the command repeatedly until all blocks are described.

| Entry offset | Width | Field |
|---|---|---|
| 0x0 | be64 | ICM virtual address of the block (MAP_ICM only; 0 for MAP_FA and MAP_ICM_AUX) |
| 0x8 | be64 | physical address of the block, OR-ed with (log2(block bytes) − 12) in the low bits |

Splitting rule (the same one Linux applies): for a physically contiguous
region, take `lg` = the index of the lowest set bit of (address OR size); `lg`
MUST be ≥ 12. Emit (size >> lg) entries of 2^lg bytes each. A driver may
instead split a region greedily into the largest naturally aligned power-of-two
blocks; both satisfy "aligned to its own size". For MAP_ICM the virtual address
advances by the block size from entry to entry and SHOULD have the same
alignment as the block.

Failure of MAP_FA or RUN_FW: issue UNMAP_FA and free the area.

Sources: `fw.c` — `mlx4_map_cmd`, `mlx4_MAP_FA`, `mlx4_RUN_FW`, `mlx4_UNMAP_FA`; `main.c` — `mlx4_load_fw`; `icm.c` — `mlx4_alloc_icm` (block allocation).

### 3.3 MOD_STAT_CFG: set the device page size

Immediately after RUN_FW, Linux sets the device page size to 4 KiB:

Input mailbox (0x100 bytes, zeroed): byte 0x02 = 1 (page-size modify enable),
byte 0x03 = 0 (log2 page size − 12, i.e. 4 KiB). op_mod 0, in_modifier 0.

A failure is only a warning in Linux.

Sources: `fw.c` — `mlx4_MOD_STAT_CFG`; `main.c` — `mlx4_init_fw`.

### 3.4 QUERY_DEV_CAP

Output mailbox. Fields a minimal Ethernet driver needs ("log" = log2; "count"
= a plain number):

| Offset | Width | Bits | Meaning | Decode |
|---|---|---|---|---|
| 0x10 | u8 | 7:0 | log max SRQ size | — |
| 0x11 | u8 | 7:0 | log max WQEs per QP | max QP size = 2^v |
| 0x12 | u8 | 3:0 | log reserved (firmware) QPs | R = 2^v |
| 0x13 | u8 | 4:0 | log max QPs | — |
| 0x14 | u8 | 7:4 | log reserved SRQs | 2^v |
| 0x15 | u8 | 4:0 | log max SRQs | — |
| 0x19 | u8 | 7:0 | log max CQEs per CQ | usable max = 2^v − 1 |
| 0x1a | u8 | 3:0 | log reserved CQs | 2^v |
| 0x1b | u8 | 4:0 | log max CQs | — |
| 0x1d | u8 | 5:0 | log max MPTs | — |
| 0x1e | u8 | 3:0 | log reserved EQs | 2^v |
| 0x1f | u8 | 3:0 | log max EQs | 2^v |
| 0x20 | u8 | 7:4 | log reserved MTT segments | 2^v |
| 0x22 | u8 | 3:0 | log reserved MPTs ("MRWs") | 2^v |
| 0x26 | be16 | 11:0 | number of "system EQs" | non-zero ⇒ SYS_EQS mode (3.7, 3.11) |
| 0x29 | u8 | 5:0 | log max RDMA requester per QP | — |
| 0x2b | u8 | 5:0 | log max RDMA responder per QP | — |
| 0x37 | u8 | 3:0 | number of ports | 1 or 2 |
| 0x38 | u8 | 4:0 | log max message size | — |
| 0x40 | be64 | 63:0 | device capability flags (table below); the dword at 0x40 is bits 63:32, the dword at 0x44 bits 31:0 | — |
| 0x48 | u8 | 7:4 | reserved UAR pages | **count** (not log) |
| 0x49 | u8 | 5:0 | UAR area size | 2^(v+20) bytes; must not exceed BAR 2 size |
| 0x4b | u8 | 7:0 | log min page size | MUST be ≤ 12 |
| 0x51 | u8 | 7:0 | max scatter/gather entries per send WQE | count |
| 0x52 | be16 | 15:0 | max send WQE size in bytes | — |
| 0x55 | u8 | 7:0 | max scatter entries per receive WQE | count |
| 0x56 | be16 | 15:0 | max receive WQE size in bytes | — |
| 0x61 | u8 | 7:0 | log max QPs per multicast group | — |
| 0x62 | u8 | 3:0 | reserved MGMs | count |
| 0x63 | u8 | 7:0 | log max multicast groups | — |
| 0x64 | u8 | 7:4 | reserved PDs | **count** |
| 0x65 | u8 | 5:0 | log max PDs | — |
| 0x68 | be32 | 31:0 | max counters (valid if flag COUNTERS) | count |
| 0x70 | be32 | see below | extended flags 2 | — |
| 0x76 | u8 | 7 | device-managed flow steering supported (unused here) | — |
| 0x80 | be16 | | RDMARC entry size (bytes) | — |
| 0x82 | be16 | | QPC entry size | — |
| 0x84 | be16 | | AUXC entry size | — |
| 0x86 | be16 | | ALTC entry size | — |
| 0x88 | be16 | | EQC entry size | — |
| 0x8a | be16 | | CQC entry size | — |
| 0x8c | be16 | | SRQC entry size | — |
| 0x8e | be16 | | cMPT entry size | — |
| 0x90 | be16 | | MTT entry (segment) size | — |
| 0x92 | be16 | | dMPT entry size | — |
| 0x94 | be32 | | BMME flags: bit 9 type-2 memory windows, bit 19 RoCE v1/v2, bit 24 **PORT_REMAP** | — |
| 0x98 | be32 | | reserved L_Key (unused) | — |
| 0xa0 | be64 | | **max ICM size** in bytes | profile total must not exceed it |

Device capability flags (64-bit value at 0x40; bit numbers of that value):

| Bit | Name | Use here |
|---|---|---|
| 7 | IPOIB_CSUM | INIT_HCA flag (optional) |
| 12 | DPDP (dual-port different protocols) | port sensing |
| 15 | LSO (Linux treats bit 15 as LSO support) | unused |
| 16 | MEM_WINDOW | INIT_HCA memory-window enable |
| 34 | FCS_KEEP | may keep FCS on receive (QP param3 bit 29) |
| 41 | VEP_UC_STEER | with bit 42: B0 steering (5.4) |
| 42 | VEP_MC_STEER | with bit 41: B0 steering; also MGID_HASH op_mod |
| 48 | COUNTERS | INIT_HCA counters enable, QP counter index |
| 52 | RSS_IP_FRAG | INIT_HCA flag (optional) |
| 55 | SENSE_SUPPORT | SENSE_PORT allowed |
| 61 | 64B_EQE | 64-byte EQEs available (not used here) |
| 62 | 64B_CQE | 64-byte CQEs available (not used here) |

Extended flags 2 (be32 at 0x70): bit 16 UPDATE_QP, bit 19 **LB_SRC_CHK**
(loopback source check, 5.10), bit 20 FSM, bit 23 **SW_CQ_INIT** (driver must
initialise CQ buffers, 4.7), bit 26 VLAN control.

After reading: if `num_sys_eqs` (0x26) is 0, set
`reserved_eqs = max(4 × reserved_UAR_count, reserved_eqs)` (each UAR page
carries four EQ doorbells, so an EQ whose doorbell falls in a reserved UAR
page cannot be used).

Linux then issues QUERY_PORT for each port (3.5).

Sources: `fw.c` — `mlx4_QUERY_DEV_CAP`; `main.c` — `mlx4_dev_cap`; `include/linux/mlx4/device.h` — `MLX4_DEV_CAP_FLAG_*`, `MLX4_BMME_FLAG_*`.

### 3.5 QUERY_PORT

in_modifier = port number (1-based). Output mailbox (command interface
revision 3):

| Offset | Width | Bits | Meaning |
|---|---|---|---|
| 0x00 | u8 | 7 | **link up** |
| | | 5 | DMFS optimized state (unused) |
| | | 4 | default sense (auto-detect port type at start) |
| | | 3 | suggested port type: 1 = Ethernet, 0 = InfiniBand |
| | | 1:0 | supported port types: bit 0 = IB, bit 1 = Ethernet |
| 0x01 | u8 | 7 | autonegotiation (as reported to ethtool) |
| | | 3:0 | IB MTU capability (unused) |
| 0x02 | be16 | | **Ethernet MTU capability** (Linux uses it as the port's maximum MTU) |
| 0x05 | u8 | | link speed code (mask 0x6f): 0x00 10G XAUI, 0x01 10G XFI, 0x02 1G, 0x04 100M, 0x08 20G, 0x40 40G, 0x20 56G, 0x0f other |
| 0x06 | u8 | 3:0 | max port width (IB) |
| 0x07 | u8 | 7:4 / 3:0 | log max GIDs / log max P_Keys (IB) |
| 0x0a | u8 | 3:0 | log max MACs (MAC table size) |
| | | 7:4 | log max VLANs |
| 0x0b | u8 | 3:0 / 7:4 | max VL / max traffic classes |
| 0x10 | be64 | 47:0 | **factory MAC address** |
| 0x18 | be32 | 31:24 / 23:0 | transceiver type / vendor OUI |
| 0x1c | be16 | | wavelength |
| 0x20 | be64 | | transceiver code |

The MAC is the low 48 bits of the be64: the most significant of those 48 bits
is the first byte on the wire (the 6 bytes 0x12..0x17 are the MAC in
transmission order).

With command interface revision 2 ("old port commands"), QUERY_PORT is not
used; the IB-style port fields come from QUERY_DEV_CAP instead. A driver MAY
refuse revision 2.

QUERY_PORT is also the way to poll link state (5.8).

Sources: `fw.c` — `mlx4_QUERY_PORT`; `en_port.c` — `mlx4_en_QUERY_PORT`; `en_port.h` — `mlx4_en_query_port_context`, speed codes; `main.c` — `_mlx4_dev_port`; `en_netdev.c` — `mlx4_en_u64_to_mac` usage.

### 3.6 QUERY_ADAPTER (optional)

Output mailbox: byte 0x10 = INTx pin number (unused); bytes 0x20 onward =
vendor-specific data. If the be16 at VSD+0x00 and VSD+0xde are both 0x05ad
(Topspin), the board ID string is at VSD+0x20; otherwise the board ID is 16
bytes at VSD+0xd0, stored with each 4-byte group byte-reversed (reverse each
dword to get the ASCII string). Informational only (Linux logs it).

Sources: `fw.c` — `mlx4_QUERY_ADAPTER`, `get_board_id`.

### 3.7 ICM and the profile

**ICM** ("InfiniHost Context Memory") is the device's context memory: tables
of QP, CQ, EQ, MPT, MTT, multicast-group and related entries. It lives in host
memory lent to the device, addressed through a device-side **ICM virtual
address space**. The driver:

1. Chooses a **profile**: how many entries of each table, and where each table
   starts in ICM virtual space.
2. Tells firmware the total ICM size (SET_ICM_SIZE), and gives it the
   auxiliary memory it asks for (MAP_ICM_AUX).
3. Backs the ICM virtual ranges it will use with host memory (MAP_ICM).
4. Passes the table bases and sizes in INIT_HCA.

Tables (entry sizes come from QUERY_DEV_CAP 0x80–0x92):

| Table | Entries | Entry size | INIT_HCA field |
|---|---|---|---|
| QPC — QP contexts | num_qps | QPC | qpc_base, log_num_qps |
| RDMARC — RDMA responder cache | num_qps × rdmarc_per_qp | RDMARC | rdmarc_base, log_rd_per_qp |
| ALTC — alternate path contexts | num_qps | ALTC | altc_base |
| AUXC — auxiliary QP contexts | num_qps | AUXC | auxc_base |
| SRQC — SRQ contexts | num_srqs | SRQC | srqc_base, log_num_srqs |
| CQC — CQ contexts | num_cqs | CQC | cqc_base, log_num_cqs |
| EQC — EQ contexts | num_eqs | EQC | eqc_base, log_num_eqs |
| dMPT — data MPT (memory regions) | num_mpts | dMPT | dmpt_base, log_mpt_sz |
| cMPT — "context" MPT | fixed: 4 × 2^24 | cMPT | cmpt_base |
| MTT — memory translation table | num_mtts | MTT | mtt_base |
| MCG — multicast/steering groups | num_mcgs | 2^log_mgm_entry_size | mc_base, log_mc_* |

**cMPT layout is fixed**: it is four sub-tables of 2^24 entries each, for
object types QP (0), SRQ (1), CQ (2), EQ (3). The cMPT entry for object `n` of
type `t` is at `cmpt_base + (t × cMPT_entry_size × 2^24) + n × cMPT_entry_size`.
Only the parts actually used are backed with memory (3.9).

**Profile algorithm** (Linux `mlx4_make_profile`, which a driver SHOULD
reproduce so the layout matches what firmware has always been given):

1. For each table: `count` = roundup_pow2(requested count);
   `bytes` = max(count × entry_size, 4096). (cMPT: count = 4 × 2^24.)
2. Sort the tables by `bytes`, largest first. Because every size is a power of
   two, placing them one after another from ICM virtual address 0 keeps each
   table naturally aligned to its own size.
3. `start` of each table = running total; total = sum of `bytes`.
4. Fail if total > max ICM size (QUERY_DEV_CAP 0xa0).

The cMPT (4 × 2^24 × entry size, e.g. 4 GiB with 64-byte entries) is
almost always the largest and lands at ICM virtual 0. That is virtual space
only.

**Recommended minimal profile** (Linux's default profile is sized for
hundreds of thousands of objects; a boot driver needs a few). Let
R = reserved QPs (QUERY_DEV_CAP 0x12).

| Parameter | Recommended value | Reason |
|---|---|---|
| base_sqpn (special QP block, 3.12) | roundup_pow2(R + 257) | same formula Linux produces when device-managed steering is off |
| num_qps | 2 × base_sqpn | room for the special block and the driver's QPs above it |
| rdmarc_per_qp | 16 (log_rd_per_qp = 4) | Linux default |
| num_srqs | roundup_pow2(reserved SRQs + 1) | SRQs are unused but the table must cover firmware's reserved ones |
| num_cqs | max(64, roundup_pow2(reserved CQs + 2)) | 2 CQs needed |
| num_eqs | non-SYS_EQS: roundup_pow2(min(max EQs, 128)); SYS_EQS: num_sys_eqs (0x26) | see note |
| num_mpts | max(64, roundup_pow2(reserved MPTs + 1)) | 1 MPT needed |
| num_mtts | roundup_pow2(first_free_mtt + 256), where first_free_mtt is from 4.1 | a few dozen entries needed |
| num_mcgs | 256 (log 8), MGM entry size 1024 (log 10) | Linux default entry size; small table |

Note on EQs: in non-SYS_EQS mode Linux's profile slot holds
roundup_pow2(min(max_eqs, 128)) entries, but it later maps 1024 EQ contexts'
worth of memory at `eqc_base` (which can run past the slot). This document
instead maps exactly the slot. In SYS_EQS mode the slot must hold at least
num_sys_eqs entries. **[HW-CHECK]** that firmware never touches EQ contexts
beyond `log_num_eqs`.

The minimum sizes firmware accepts in INIT_HCA are not documented in the
sources. **[HW-CHECK]** the small profile; if INIT_HCA fails with status 0x03
or 0x08, fall back towards Linux's defaults (num_qps 2^18, num_srqs 2^16,
num_cqs 2^16, num_mcgs 2^13, num_mpts 2^19, num_mtts 2^20) or its "low memory"
profile (2^17 QPs, 2^6 SRQs, 2^8 CQs, 2^8 MCGs, 2^9 MPTs, 2^7 MTTs).

Sources: `profile.c` — `mlx4_make_profile`; `main.c` — `default_profile`, `low_mem_profile`, `mlx4_init_hca`, `mlx4_init_cmpt_table`; `mlx4.h` — `MLX4_CMPT_TYPE_*`, `MLX4_CMPT_SHIFT`, `MLX4_NUM_CMPTS`; `mcg.c` — `mlx4_get_mgm_entry_size`.

### 3.8 SET_ICM_SIZE and MAP_ICM_AUX

1. SET_ICM_SIZE: in_param (immediate) = total ICM size in bytes from the
   profile. The immediate output (HCR 0x0c/0x10) = number of 4 KiB
   **auxiliary pages** firmware needs.
2. Allocate that many pages (same block rules as MAP_FA, 3.2), describe them
   with the page-list mailbox (virtual address fields 0), and issue
   MAP_ICM_AUX.

On failure after MAP_ICM_AUX: UNMAP_ICM_AUX, free.

Sources: `fw.c` — `mlx4_SET_ICM_SIZE`; `icm.c` — `mlx4_MAP_ICM_AUX`, `mlx4_UNMAP_ICM_AUX`; `main.c` — `mlx4_init_icm`.

### 3.9 MAP_ICM: backing the tables

MAP_ICM gives host memory to a range of ICM virtual addresses. Mailbox: page
list (3.2) with the virtual address field filled in. Blocks must be at least
4 KiB and naturally aligned (physical); give the virtual ranges the same
alignment.

Linux maps table memory lazily in 256 KiB chunks as objects are created, but
maps firmware-reserved objects up front. Mapping everything up front is
equivalent and simpler. **Recommended**: before INIT_HCA, map:

| Range (ICM virtual) | Bytes |
|---|---|
| QPC, RDMARC, ALTC, AUXC, SRQC, CQC, EQC, dMPT, MTT, MCG | each whole table (its profile `bytes`) |
| cMPT, QP sub-table: `cmpt_base + 0 × S` | num_qps × cMPT entry size, rounded up to 4 KiB |
| cMPT, SRQ sub-table: `cmpt_base + 1 × S` | num_srqs × cMPT entry size, rounded up |
| cMPT, CQ sub-table: `cmpt_base + 2 × S` | num_cqs × cMPT entry size, rounded up |
| cMPT, EQ sub-table: `cmpt_base + 3 × S` | num_eqs × cMPT entry size, rounded up |

where S = cMPT entry size × 2^24.

Memory given to ICM must be DMA-able and zeroed. The driver writes into ICM
only in one place: MTT entries (4.2). Remember, per range, the host virtual
and physical address, so the driver can find the MTT entries and so teardown
can UNMAP_ICM each range.

Linux order: SET_ICM_SIZE, MAP_ICM_AUX, cMPT (QP, SRQ, CQ, EQ), EQC, MTT,
dMPT, QPC, AUXC, ALTC, RDMARC, CQC, SRQC, MCG, then INIT_HCA.

Sources: `icm.c` — `mlx4_MAP_ICM`, `mlx4_init_icm_table`, `mlx4_table_get`, `mlx4_UNMAP_ICM`; `main.c` — `mlx4_init_icm`, `mlx4_init_cmpt_table`.

### 3.10 INIT_HCA

Input mailbox, 0x200 bytes, zeroed, then:

| Offset | Width | Value |
|---|---|---|
| 0x000 | u8 | INIT_HCA version = **2** |
| 0x00e | u8 | cache line: ((log2(CPU cache line bytes) − 4) << 5) \| 0x10. For 64-byte lines: 0x50 |
| 0x014 | be32 | flags (below) |
| 0x030 | be64 | qpc_base (ICM virtual) |
| 0x037 | u8 | log_num_qps (this byte overlays the low byte of qpc_base, which is 0 because the base is ≥ 4 KiB aligned) |
| 0x048 | be64 | srqc_base |
| 0x04f | u8 | log_num_srqs |
| 0x050 | be64 | cqc_base |
| 0x057 | u8 | log_num_cqs |
| 0x058 | be32 | bit 29: 64-byte EQEs; bit 30: 64-byte CQEs. **Leave 0** (32-byte entries, 4.6/4.7) |
| 0x05b | u8 | EQE/CQE stride (leave 0) |
| 0x060 | be64 | altc_base |
| 0x070 | be64 | auxc_base |
| 0x080 | be64 | eqc_base |
| 0x087 | u8 | log_num_eqs; **0x1f** in SYS_EQS mode |
| 0x08a | be16 | num_sys_eqs (SYS_EQS mode only; else 0) |
| 0x090 | be64 | rdmarc_base |
| 0x097 | u8 | log_rd_per_qp |
| 0x0c0 | be64 | mc_base (MCG table) |
| 0x0d3 | u8 | log2 MGM entry size (10) |
| 0x0d7 | u8 | log2 MGM hash size = log2(num_mcgs) − 1 |
| 0x0d8 | u8 | bit 3: unicast steering enable — set for **B0 steering** (5.4), clear for A0 |
| 0x0db | u8 | log2(num_mcgs) |
| 0x0f0 | be64 | dmpt_base |
| 0x0f8 | u8 | bit 7: memory windows enable — set if capability bit 16 or BMME bit 9 (as Linux) |
| 0x0fb | u8 | log2(num_mpts) |
| 0x100 | be64 | mtt_base |
| 0x108 | be64 | cmpt_base |
| 0x12a | u8 | log2(number of UAR pages) = log2(UAR area bytes / 4096) |
| 0x12b | u8 | UAR page size: log2(bytes) − 12 = **0** |

Flags dword at 0x014:

| Bit | Meaning | Set? |
|---|---|---|
| 0 | check port for UD address vectors | always (Linux) |
| 1 | host is big-endian | **clear** on x86/little-endian |
| 2 | enhanced QoS | clear |
| 3 | IPoIB checksum | optional (Linux sets it if capability bit 7) |
| 4 | enable counters | set if capability bit 48 (COUNTERS) — see 4.8.4 counter index |
| 6 | device-managed flow steering | clear |
| 13 | RSS spread to IP fragments | optional (Linux sets it if capability bit 52) |

Not used here: 0x00c VXLAN parser, 0x018 bit 31 recoverable-error events,
0x140 driver version string, 0x1d0 flow-steering block.

INIT_HCA uses timeout class C (60 s). On failure: unmap everything (3.13).

Sources: `fw.c` — `mlx4_INIT_HCA`; `main.c` — `mlx4_init_hca`, `choose_steering_mode`.

### 3.11 SYS_EQS mode: QUERY_FUNC

If QUERY_DEV_CAP reported num_sys_eqs ≠ 0, issue QUERY_FUNC (in_modifier 0)
after INIT_HCA. Output mailbox: be16 at 0x04 = reserved EQs, be16 at 0x06 =
max EQs, byte 0x0b bits 3:0 = reserved UARs. Use these reserved-EQ and max-EQ
values instead of QUERY_DEV_CAP's.

Sources: `fw.c` — `mlx4_QUERY_FUNC`; `main.c` — `mlx4_query_func`, `mlx4_init_hca`.

### 3.12 CONF_SPECIAL_QP

Linux always reserves an 8-aligned block of 8 "special" QP numbers (for the
InfiniBand management QPs) and tells firmware where it is: CONF_SPECIAL_QP
with in_modifier = base QPN, right after the QP number space is set up
(after INIT_HCA and EQ creation). At teardown, CONF_SPECIAL_QP with 0.

An Ethernet-only driver probably does not need it, but issuing it keeps the
firmware state identical to Linux. **Recommended**: issue it with
`base_sqpn` from 3.7, and never use QPNs in [base_sqpn, base_sqpn + 8) for
anything else. **[HW-CHECK]** whether it can be omitted.

Sources: `qp.c` — `mlx4_CONF_SPECIAL_QP`, `mlx4_init_qp_table`, `mlx4_cleanup_qp_table`.

### 3.13 Teardown and its ordering

Linux's complete unload order (commands only):

1. CLOSE_PORT for each port brought up.
2. Data path: 2RST_QP on each QP; HW2SW_CQ on each CQ; steering detach.
3. HW2SW_MPT on the memory region.
4. CONF_SPECIAL_QP with 0.
5. MAP_EQ with unmap bit set (same event mask); HW2SW_EQ on each EQ.
6. CLOSE_HCA (op_mod 0; op_mod 1 is "panic" close and is not used here),
   timeout class C.
7. UNMAP_ICM for every mapped range: in_param = ICM virtual address of the
   range, in_modifier = number of 4 KiB pages. Linux unmaps in the reverse
   order of mapping (MCG, SRQC, CQC, RDMARC, ALTC, AUXC, QPC, dMPT, MTT, EQC,
   cMPT EQ/CQ/SRQ/QP).
8. UNMAP_ICM_AUX; free the auxiliary pages.
9. UNMAP_FA; free the firmware area.
10. Release ownership (1.5).

After UNMAP_FA the firmware no longer uses host memory. **Host memory lent to
the device (firmware area, auxiliary pages, ICM) must not be reused until
UNMAP_FA has completed** (or the device has been reset, 1.6).

Sources: `main.c` — `mlx4_unload_one`, `mlx4_close_hca`, `mlx4_close_fw`, `mlx4_free_icms`, `mlx4_free_ownership`; `en_netdev.c` — `mlx4_en_stop_port`; `eq.c` — `mlx4_cleanup_eq_table`, `mlx4_free_eq`; `fw.c` — `mlx4_CLOSE_HCA`, `mlx4_CLOSE_PORT`.

### 3.14 The minimal bring-up order (summary)

1. PCI: enable memory space and bus master; disable INTx (1.7).
2. Claim ownership (1.5).
3. Reset (1.6).
4. Initialise the command interface: `toggle` (2.4), one mailbox.
5. QUERY_FW (3.1); check interface revision.
6. Allocate the FW area; MAP_FA; RUN_FW (3.2).
7. MOD_STAT_CFG, 4 KiB pages (3.3).
8. QUERY_DEV_CAP (3.4); QUERY_PORT for each port (3.5).
9. Choose the steering mode (5.4) and the profile (3.7).
10. SET_ICM_SIZE; allocate aux pages; MAP_ICM_AUX (3.8).
11. MAP_ICM for every table (3.9).
12. INIT_HCA (3.10).
13. If SYS_EQS: QUERY_FUNC (3.11).
14. Optional: QUERY_ADAPTER (3.6).
15. Create the objects of section 4 and bring up the port (section 5; the
    complete order is in section 6).

Linux reference order: `mlx4_load_one` → `mlx4_reset` → `mlx4_cmd_init` →
`mlx4_init_fw` (QUERY_FW, `mlx4_load_fw`, MOD_STAT_CFG) → QUERY_DEV_CAP →
`mlx4_init_hca` (profile, `mlx4_init_icm`, INIT_HCA, QUERY_FUNC,
QUERY_ADAPTER) → `mlx4_setup_hca` (UAR, PD, MR table, MCG table, EQs, CQ/QP
tables incl. CONF_SPECIAL_QP, counters) → `mlx4_en` (PD, UAR, MR, per-port
start: CQs, MAC registration, QPs, SET_PORT, INIT_PORT, steering).

Sources: `main.c` — `mlx4_load_one`, `mlx4_init_fw`, `mlx4_init_hca`, `mlx4_setup_hca`; `en_main.c` — `mlx4_en_probe`; `en_netdev.c` — `mlx4_en_start_port`.

---

## 4. Objects and context tables for one Ethernet port

A minimal driver creates, per port: one PD, one memory region (MPT), one EQ,
two CQs (receive, send), two QPs (receive, send), doorbell records and queue
buffers, and uses one UAR page.

### 4.1 Object numbers and reserved ranges

Firmware reserves the low numbers of most object types; the driver picks
numbers above them. Linux's allocators hand out the lowest free number above
the reserved range first, so a driver that takes "the first free" gets the
numbers Linux's first allocation would.

| Object | Reserved range (never use) | Minimal driver's choice |
|---|---|---|
| QPN | [0, R) firmware; [base_sqpn, base_sqpn+8) special block | RX QPN = base_sqpn + 128; TX QPN = base_sqpn + 256 (see 5.4 for why these are 128-aligned and separated) |
| CQN | [0, reserved CQs) | RX CQN = reserved CQs; TX CQN = reserved CQs + 1 |
| EQN | [0, reserved EQs) (3.4 adjustment, or QUERY_FUNC in SYS_EQS mode) | EQN = reserved EQs; must be < num_eqs and < 256 (CQ contexts store the EQN in one byte) |
| MPT index | [0, reserved MPTs) | reserved MPTs |
| MTT index | [0, first_free_mtt), see below | allocate upward from first_free_mtt |
| PD | [0, reserved PDs) (count from 0x64) | PD = reserved PDs; PDs are 17-bit values |
| UAR page | [0, max(128, reserved UAR count)) | max(128, reserved UAR count) |
| MCG index | none for hashed entries; AMGM (overflow) entries are [num_mcgs/2, num_mcgs) | as the hash says (5.4) |
| Counter | none; Linux gives port 1 counter 0, port 2 counter 1 | port − 1 (only with COUNTERS enabled) |

**MTT reserved area.** QUERY_DEV_CAP gives `reserved_mtts` = 2^(0x20[7:4]). Linux
rounds the reserved byte span up to the CPU cache line (firmware writes those
entries; the driver writes the others): `aligned = ceil(reserved_mtts ×
MTT_entry_size / cache_line) × cache_line / MTT_entry_size`, then claims the
first 2^ceil(log2(aligned)) entries. So `first_free_mtt =
roundup_pow2(aligned)`.

Sources: `main.c` — `mlx4_dev_cap`, `mlx4_init_icm` (MTT alignment); `mr.c` — `mlx4_init_mr_table`, `mlx4_mr_alloc`; `pd.c` — `mlx4_init_pd_table`, `mlx4_init_uar_table`; `eq.c` — `mlx4_init_eq_table`; `cq.c` — `mlx4_init_cq_table`; `qp.c` — `mlx4_init_qp_table`; `mlx4.h` — `NOT_MASKED_PD_BITS`.

### 4.2 MTT: memory translation entries

An MTT entry maps one device page of a queue buffer to its physical address.
Every queue (EQ, CQ, QP) names a run of consecutive MTT entries by the byte
offset of the first one in the MTT table, plus the page size.

- **Entry format**: be64 = physical address of the page, OR-ed with bit 0 =
  1 ("present"). The page address must be aligned to the page size.
- **Entry size**: QUERY_DEV_CAP 0x90 (8 bytes expected; Linux treats it as the
  size of an "MTT segment" of 1 entry).
- **Addressing a run in a context**: `mtt_offset` = first entry index ×
  MTT entry size (a byte offset within the MTT table, not an ICM address).
  Contexts store it as `mtt_base_addr_h` (bits 39:32, one byte) and
  `mtt_base_addr_l` (bits 31:0, be32).
- **Page size in a context**: `log_page_size` = log2(page bytes) − 12.
- **Allocation**: Linux allocates runs from a buddy allocator, so a run of n
  entries starts at a multiple of roundup_pow2(n). **Recommended**: keep that
  alignment.
- **Recommended page size: 4 KiB** for every queue buffer (one MTT entry per
  4 KiB). Linux uses 4 KiB pages for EQs and for multi-page buffers; `mlx4_en`
  also uses a single MTT entry covering a whole naturally-aligned buffer (page
  size = buffer size). Either is valid.

**Writing entries.** In native (non-virtualised) mode Linux writes MTT
entries **directly into ICM memory**: it finds the host address of entry `i`
from the MTT table's ICM mapping (`mtt_base` virtual → host page backing it)
and stores the big-endian values there. A UEFI driver does the same: since it
mapped the MTT table itself (3.9), entry `i` is at host address
(MTT backing memory) + i × entry size. Write entries before creating the object
that uses them; on x86 the memory is cache-coherent, so a store fence before
the create command suffices.

Alternative: the WRITE_MTT command (0x011). Input mailbox: be64 at 0x00 =
first MTT index, be64 at 0x08 = 0, then be64 entries (address | 1) from 0x10;
in_modifier = entry count (≤ 510 per command). Linux only sends this in
virtualised mode, where the host driver emulates it with direct writes, so
whether ConnectX-3 firmware implements it natively is unknown **[HW-CHECK]**.
Prefer direct writes.

Sources: `mr.c` — `mlx4_mtt_init`, `mlx4_mtt_addr`, `__mlx4_alloc_mtt_range`, `mlx4_write_mtt`, `__mlx4_write_mtt`, `mlx4_write_mtt_chunk`, `mlx4_buf_write_mtt`; `include/linux/mlx4/device.h` — `MLX4_MTT_FLAG_PRESENT`; `alloc.c` — `mlx4_buf_direct_alloc`, `mlx4_alloc_hwq_res`; `icm.c` — `mlx4_table_find`.

### 4.3 Memory region (MPT) and L_Key

Every data segment in a WQE carries an L_Key naming a memory region. `mlx4_en`
registers **one physical memory region spanning all of memory**: addresses in
WQEs are then plain physical (DMA) addresses. Do the same.

SW2HW_MPT: in_modifier = MPT index (4.1) masked to num_mpts − 1; input mailbox
= a 64-byte dMPT entry:

| Offset | Width | Field | Value |
|---|---|---|---|
| 0x00 | be32 | flags | 0xF0000000 (SW-owns state, bits 31:28 = 0xF) \| 0x00020000 (MIO, bit 17) \| 0x00000200 (PHYSICAL, bit 9) \| 0x00000100 (REGION, bit 8) \| 0x00000400 (local read, bit 10) \| 0x00000800 (local write, bit 11) = **0xF0020F00** |
| 0x04 | be32 | qpn | 0 |
| 0x08 | be32 | key | MPT index (the "hardware index" form of the key) |
| 0x0c | be32 | pd_flags | PD (bits 16:0) \| 0x03000000 (EN_INV, bits 25:24) |
| 0x10 | be64 | start | 0 |
| 0x18 | be64 | length | 0xFFFFFFFFFFFFFFFF |
| 0x20 | be32 | lkey | 0 |
| 0x24 | be32 | win_cnt | 0 |
| 0x2b | u8 | mtt_rep | 0 |
| 0x2c | be64 | mtt_addr | 0 (physical region; note: unaligned 64-bit field) |
| 0x34 | be32 | mtt_sz | 0 |
| 0x38 | be32 | entity_size | 12 (page shift; Linux passes the 4 KiB ICM page shift for an MTT-less region) |
| 0x3c | be32 | first_byte_offset | 0 |

Access flag bits available (in `flags`): 10 local read, 11 local write,
12 remote read, 13 remote write, 14 atomic, 15 bind memory window.

**L_Key** to put in WQEs: rotate the 32-bit MPT index left by 8 bits —
`lkey = (index << 8) | (index >> 24)`. For an index below 2^24 this is simply
`index << 8`. (The key written in the dMPT entry at 0x08 is the index itself.)

Teardown: HW2SW_MPT, op_mod 1, in_param 0, in_modifier = MPT index (no output
mailbox).

The special key 0x00000100 ("invalid L_Key", also used as the "padding"
memory type in receive descriptors) must never be a real region's L_Key; with
MPT index 1 it would be, so if `reserved MPTs` were 1 skip to index 2.
**[HW-CHECK]** (firmware normally reserves more than one MPT).

Sources: `mr.c` — `mlx4_mr_alloc`, `mlx4_mr_alloc_reserved`, `mlx4_mr_enable`, `hw_index_to_key`, `key_to_hw_index`, `mlx4_SW2HW_MPT`, `mlx4_HW2SW_MPT`, `mlx4_mr_free`; `mlx4.h` — `mlx4_mpt_entry`, `MLX4_MPT_FLAG_*`, `MLX4_MPT_PD_FLAG_*`; `include/linux/mlx4/device.h` — `MLX4_PERM_*`; `include/linux/mlx4/qp.h` — `MLX4_INVALID_LKEY`; `en_main.c` — `mlx4_en_probe`.

### 4.4 Doorbell records (host memory)

Some doorbells are not MMIO writes but values the device reads from host
memory by DMA ("doorbell records"):

| Record | Size | Contents | Named by |
|---|---|---|---|
| CQ doorbell record | 8 bytes, 8-byte aligned | dword 0: be32 consumer index (bits 23:0) — the "set CI" record; dword 1: be32 arm record (only for arming; keep 0) | CQ context `db_rec_addr` |
| QP receive doorbell record | 4 bytes (allocate 8, 8-byte aligned) | be32 receive-queue producer counter, bits 15:0 | QP context `db_rec_addr` |

Linux allocates doorbell records from 4 KiB coherent pages, 4-byte slots, in
pairs for CQs. Initialise every record to 0 before creating the object.

Sources: `alloc.c` — `mlx4_db_alloc`, `mlx4_alloc_db_from_pgdir`, `mlx4_alloc_hwq_res`; `include/linux/mlx4/cq.h` — `mlx4_cq_set_ci`, `mlx4_cq_arm`; `en_cq.c` — `mlx4_en_activate_cq`; `en_rx.c` — `mlx4_en_update_rx_prod_db`.

### 4.5 Queue buffers

| Queue | Entry size | Buffer | Notes |
|---|---|---|---|
| EQ | 32 bytes | nent × 32, nent a power of two | 4.6 |
| CQ | 32 bytes | nent × 32, nent a power of two | 4.7 |
| QP | SQ: 64-byte TXBBs; RQ: stride 16 × 2^k | SQ bytes + RQ bytes, order by stride (4.8.2) | 4.8 |

All queue buffers: DMA-able, page-aligned (4 KiB), zeroed unless stated
otherwise, described by MTT entries (4.2), and never freed while the object is
owned by hardware.

### 4.6 Event queue (EQ)

A CQ context must name an EQ, so at least one EQ exists even without
interrupts. Linux creates an "async" EQ first and maps the asynchronous event
types to it; with polling the driver may read it to learn of port changes and
errors.

**Size**: Linux's async EQ has 0x100 entries plus 0x80 spare, rounded to a
power of two (512). The consumer index must be reported to the device at least
every 0x80 entries processed, or the device considers the EQ overflowed.
**Recommended**: 256 entries (8 KiB, two 4 KiB MTT entries), process at most
0x80 at a time between consumer-index updates.

**Initialise the buffer** so every entry is owned by hardware for the first
pass: set bit 7 of byte 0x1f of every 32-byte entry (fill the buffer with 0xff,
or set byte 0x1f of each entry to 0x80). Linux does not do this (it relies on
the buffer being zeroed and presumably on firmware initialising it at
SW2HW_EQ), so doing it is harmless and protects against the alternative.
**[HW-CHECK]**.

**SW2HW_EQ**: in_modifier = EQN, input mailbox = 64-byte EQ context (other
bytes 0):

| Offset | Width | Field | Value |
|---|---|---|---|
| 0x00 | be32 | flags | status (bits 31:28) = 0 OK; state (bits 11:8) = 0x9 ARMED → **0x00000900**. Other defined bits: 18 EC, 17 OI (overflow ignore), 24 owner; not set by Linux |
| 0x0a | be16 | page_offset | 0 |
| 0x0c | u8 | log_eq_size | log2(nent) |
| 0x11 | u8 | eq_period | 0 |
| 0x13 | u8 | eq_max_count | 0 |
| 0x17 | u8 | intr | interrupt vector: 0 (no interrupts used) |
| 0x18 | u8 | log_page_size | log2(page) − 12 (0 for 4 KiB) |
| 0x1b | u8 | mtt_base_addr_h | MTT byte offset bits 39:32 |
| 0x1c | be32 | mtt_base_addr_l | MTT byte offset bits 31:0 |
| 0x28 | be32 | consumer_index | 0 |
| 0x2c | be32 | producer_index | 0 |

Other defined state values: 0xA FIRED, 0xB ALWAYS_ARMED. Status 0xA = write
failure.

**MAP_EQ**: in_param (immediate 64-bit) = event-type mask (bit n = event type
n, Appendix D), in_modifier = EQN (bit 31 = 1 to unmap). Linux maps: path
migration (0x01), communication established (0x02), SQ drained (0x03), CQ
error (0x04), WQ catastrophic (0x05), EEC catastrophic (0x06), path migration
failed (0x07), WQ invalid request (0x10), WQ access error (0x11), SRQ
catastrophic (0x12), SRQ last WQE (0x13), SRQ limit (0x14), port change
(0x09), ECC (0x0e), command (0x0a), operation required (0x1a), comm channel
(0x18), FLR (0x1c), fatal warning (0x1b); plus port-management change (0x1d)
if capability bit 59. **Recommended** for a polling driver: map at least port
change (0x09), CQ error (0x04), WQ catastrophic (0x05), WQ invalid request
(0x10), WQ access error (0x11) and local catastrophic (0x08); do **not** map
command completion (0x0a) since commands are polled. Failure of MAP_EQ is only
a warning in Linux.

**EQE format** (32 bytes):

| Offset | Width | Field |
|---|---|---|
| 0x00 | u8 | reserved |
| 0x01 | u8 | event type (Appendix D) |
| 0x02 | u8 | reserved |
| 0x03 | u8 | event subtype |
| 0x04 | 24 bytes | event data (below) |
| 0x1c | u8 | slave ID (multi-function) |
| 0x1f | u8 | bit 7: **owner** |

Event data by type: completion (0x00): be32 at 0x04, bits 23:0 = CQN. QP events
(0x01–0x03, 0x05, 0x07, 0x10, 0x11, 0x13): be32 at 0x04 bits 23:0 = QPN. CQ
error (0x04): be32 at 0x04 bits 23:0 = CQN, byte 0x0f = syndrome (1 = overrun,
else access violation). Port change (0x09): be32 at 0x0c, bits 31:28 = port
number; subtype 1 = link down, 4 = link active. Command (0x0a): be16 at 0x06
token, byte 0x0f status, be64 at 0x10 output parameter.

**Polling an EQ.** Keep a free-running consumer counter `ci` (starts at 0).
The entry at index (ci mod nent) is valid (owned by software) when its owner
bit equals ((ci / nent) mod 2), i.e. owner == 0 on the first pass, 1 on the
second, and so on. After reading the owner bit, issue a read barrier before
reading the rest of the entry. After consuming entries, write the doorbell:
32-bit big-endian value `(ci mod 2^24) | (req_not << 31)` to UAR page (EQN / 4),
offset 0x800 + 8 × (EQN mod 4) (1.4); `req_not` = 1 would re-arm the EQ to
interrupt, so a polling driver writes 0. Linux arms the async EQ once after
creating it (write with bit 31 set); with interrupts disabled at PCI level that
is harmless, and it is not required.

**Teardown**: MAP_EQ with the same mask and bit 31 of in_modifier set; then
HW2SW_EQ (op_mod 1, in_param 0, in_modifier EQN, no mailbox).

Sources: `eq.c` — `mlx4_create_eq`, `mlx4_free_eq`, `mlx4_MAP_EQ`, `mlx4_SW2HW_EQ`, `mlx4_HW2SW_EQ`, `mlx4_get_eq_uar`, `eq_set_ci`, `get_eqe`, `next_eqe_sw`, `mlx4_eq_int`, `mlx4_init_eq_table`, `get_async_ev_mask`, `MLX4_EQ_*`; `mlx4.h` — `mlx4_eq_context`; `include/linux/mlx4/device.h` — `mlx4_eqe`, `MLX4_EVENT_TYPE_*`, `MLX4_PORT_CHANGE_SUBTYPE_*`.

### 4.7 Completion queue (CQ)

One CQ for receive completions and one for send completions (Linux uses one
per ring).

**Size**: a power of two; at least the number of WQEs that can be
outstanding on the queues it serves (receive: RX ring size; send: TX ring size
in TXBBs is a safe bound). Max from QUERY_DEV_CAP 0x19 (2^v − 1 usable).

**Initialise the buffer**: fill with 0xCC bytes ("the initialisation value
required by the firmware"; bit 7 of byte 0x1f is then 1 = hardware-owned for
pass 0). Linux fills with 0xCC only when capability SW_CQ_INIT (0x70 bit 23)
is set, and otherwise relies on firmware initialising CQEs at SW2HW_CQ (the
`mlx4_en` receive CQ is additionally re-stamped with owner = 1 after
creation). Filling always is harmless. **[HW-CHECK]**.

**SW2HW_CQ**: in_modifier = CQN; op_mod = **1 if SW_CQ_INIT** is set (tells
firmware the driver initialised the entries), else 0. Input mailbox = 64-byte
CQ context (other bytes 0):

| Offset | Width | Field | Value |
|---|---|---|---|
| 0x00 | be32 | flags | 0 (bit 18 = collapsed CQ, bit 19 = timestamps; both 0) |
| 0x0a | be16 | page_offset | 0 |
| 0x0c | be32 | logsize_usrpage | (log2(nent) << 24) \| UAR page index (bits 23:0) |
| 0x10 | be16 | cq_period | 0 (moderation, unused) |
| 0x12 | be16 | cq_max_count | 0 |
| 0x17 | u8 | comp_eqn | EQN that receives completion events (4.6) |
| 0x18 | u8 | log_page_size | log2(page) − 12 |
| 0x1b | u8 | mtt_base_addr_h | MTT byte offset bits 39:32 |
| 0x1c | be32 | mtt_base_addr_l | MTT byte offset bits 31:0 |
| 0x20 | be32 | last_notified_index | 0 |
| 0x24 | be32 | solicit_producer_index | 0 |
| 0x28 | be32 | consumer_index | 0 |
| 0x2c | be32 | producer_index | 0 |
| 0x38 | be64 | db_rec_addr | physical address of the 8-byte CQ doorbell record (4.4) |

Other CQ status/state values: status 9 overflow, 0xA write fail; state 9
armed, 6 armed-solicited (bits 11:8).

**CQE format** (32 bytes; normal completion):

| Offset | Width | Field |
|---|---|---|
| 0x00 | be32 | vlan_my_qpn: bits 23:0 = QPN the completion belongs to; bit 29 = C-VLAN present, bit 30 = S-VLAN present (only when stripping) |
| 0x04 | be32 | immed_rss_invalid (RSS hash; unused) |
| 0x08 | be32 | g_mlpath_rqpn (unused for Ethernet) |
| 0x0c | be16 | sl_vid: bits 11:0 = stripped VLAN ID (only when stripping) |
| 0x0e | be16 | rlid (IB) / start of source MAC for some modes |
| 0x10 | be16 | status: bit 6 IPv4, 7 IPv4 fragment, 8 IPv6, 9 IPv4 options, 10 TCP, 11 UDP, 12 IP checksum OK |
| 0x12 | u8 | ipv6_ext_mask |
| 0x13 | u8 | badfcs_enc: bit 4 = bad FCS, bit 0 = LLC, bit 1 = SNAP, bit 2 = L4 checksum OK |
| 0x14 | be32 | **byte_cnt**: received frame length (receive completions) |
| 0x18 | be16 | **wqe_index**: index of the completed WQE (receive: RQ slot; send: TXBB index of the WQE) |
| 0x1a | be16 | checksum |
| 0x1f | u8 | owner_sr_opcode: bit 7 **owner**; bit 6 **is-send** (1 = send completion); bits 4:0 **opcode** |

Error CQE (opcode 0x1e): be32 at 0x00 = QPN, be16 at 0x18 = WQE index, byte
0x1a = vendor syndrome, byte 0x1b = syndrome (Appendix C), byte 0x1f as above.

Opcodes: send completion of SEND = 0x0a; receive completion of SEND = 0x01
(0x00 RDMA write with immediate, 0x02 send with immediate, 0x03 send with
invalidate); **0x1e = error**; 0x16 = resize.

**Polling a CQ.** Free-running consumer counter `ci` from 0. Entry
(ci mod nent) is valid when (owner bit) == ((ci / nent) mod 2). After seeing a
valid owner bit, read barrier, then read the entry. After consuming entries,
write `ci mod 2^24` (be32) into dword 0 of the CQ doorbell record (4.4). The
device reads that record to know which entries are free; if it is not updated
the CQ overflows (a CQ error event, syndrome 1).

Arming (only for interrupts; not used): write arm record and the 64-bit CQ
doorbell at UAR + 0x20 = [be32 (sn<<28 | cmd | CQN), be32 ci] with cmd
0x02000000 "request notification" or 0x01000000 "solicited only".

**Teardown**: HW2SW_CQ, op_mod 1, in_param 0, in_modifier CQN (no mailbox).
Destroy CQs only after the QPs using them are in RESET.

Sources: `cq.c` — `mlx4_cq_alloc`, `mlx4_cq_free`, `mlx4_SW2HW_CQ`, `mlx4_HW2SW_CQ`, `mlx4_init_kernel_cqes`, `MLX4_CQ_*`; `mlx4.h` — `mlx4_cq_context`; `include/linux/mlx4/cq.h` — `mlx4_cqe`, `mlx4_err_cqe`, `MLX4_CQE_*`, `mlx4_cq_arm`, `mlx4_cq_set_ci`; `en_cq.c` — `mlx4_en_create_cq`, `mlx4_en_activate_cq`; `en_rx.c` — `mlx4_en_process_rx_cq`; `en_tx.c` — `mlx4_en_process_tx_cq`; `en_netdev.c` — `mlx4_en_start_port` (owner re-stamp); `include/linux/mlx4/device.h` — `MLX4_OPCODE_*`, `MLX4_RECV_OPCODE_*`, `MLX4_CQE_OPCODE_*`.

### 4.8 Queue pair (QP)

An Ethernet QP in `mlx4_en` has **service type MLX (0x7)**, i.e. "raw
Ethernet": the send queue transmits whole Ethernet frames supplied by the
driver and the receive queue receives whole frames. `mlx4_en` uses separate
QPs for receive and send; a minimal driver does the same:

- **RX QP**: a receive queue of N descriptors plus a dummy 1-TXBB send queue.
- **TX QP**: a send queue of M TXBBs and no real receive queue.

#### 4.8.1 QP context (0xF8 bytes)

The QP context appears in the modify-QP mailbox at offset 0x08 (4.8.3). Offsets
below are from the start of the context.

| Offset | Width | Field |
|---|---|---|
| 0x00 | be32 | flags: bits 31:28 = **next state** (0 RST, 1 INIT, 2 RTR, 3 RTS, 4 SQER, 5 SQD, 6 ERR); bits 23:16 = **service type** (0x7 MLX); bits 12:11 = path-migration state; bit 13 = RSS indirection QP |
| 0x04 | be32 | pd (PD number) |
| 0x08 | u8 | mtu_msgmax: bits 7:5 path MTU, bits 4:0 log max message; `mlx4_en` writes **0xff** (MTU code 7, msgmax 31) |
| 0x09 | u8 | rq_size_stride: bits 6:3 = log2(RQ entries), bits 2:0 = log2(RQ stride bytes) − 4 |
| 0x0a | u8 | sq_size_stride: bits 6:3 = log2(SQ TXBBs), bits 2:0 = log2(SQ stride) − 4 (stride 64 → 2); bit 7 = no SQ prefetch |
| 0x0b | u8 | rlkey_roce_mode |
| 0x0c | be32 | usr_page: UAR page index whose send doorbell (0x14) this QP uses |
| 0x10 | be32 | local_qpn (overwritten with the QPN by Linux in the mailbox) |
| 0x14 | be32 | remote_qpn |
| 0x18 | 0x2c | primary path (table below) |
| 0x44 | 0x2c | alternate path (0) |
| 0x70 | be32 | params1 (RC/UC only; 0) |
| 0x78 | be32 | next_send_psn |
| 0x7c | be32 | **cqn_send**: bits 23:0 = CQN for send completions |
| 0x80 | be16 | roce_entropy |
| 0x88 | be32 | last_acked_psn |
| 0x8c | be32 | ssn |
| 0x90 | be32 | params2: bit 3 = FPP ("force physical port", see below); bits 15:13 remote access (0) |
| 0x94 | be32 | rnr_nextrecvpsn |
| 0x98 | be32 | xrcd |
| 0x9c | be32 | **cqn_recv**: bits 23:0 = CQN for receive completions |
| 0xa0 | be64 | **db_rec_addr**: physical address of the receive doorbell record |
| 0xa8 | be32 | qkey |
| 0xac | be32 | srqn: bit 24 = uses SRQ, bits 23:0 SRQN; bits 30:28 = tunnel receive mode (only with VXLAN offload, 0 here) |
| 0xb0 | be32 | msn |
| 0xb4 | be16 | rq_wqe_counter |
| 0xb6 | be16 | sq_wqe_counter |
| 0xbc | be16 | rate_limit_params |
| 0xbf | u8 | qos_vport |
| 0xc0 | be32 | **param3**: bit 30 = VLAN stripping **disable** (VSD); bit 29 = keep FCS on receive |
| 0xc4 | be32 | nummmcpeers_basemkey |
| 0xc8 | u8 | log_page_size: log2(page) − 12 for the QP buffer's MTT entries |
| 0xcb | u8 | mtt_base_addr_h: MTT byte offset bits 39:32 |
| 0xcc | be32 | mtt_base_addr_l: MTT byte offset bits 31:0 |
| 0xd0 | 40 bytes | reserved |

Primary path (0x2c bytes, at context 0x18):

| Path offset | Context offset | Width | Field |
|---|---|---|---|
| 0x00 | 0x18 | u8 | fl: bit 6 CV, bit 5 SV, bit 2 hide CQE VLAN, bit 1 **Ethernet source check, multicast loopback**, bit 0 source check, unicast loopback |
| 0x01 | 0x19 | u8 | control / vlan_control: bit 7 = **source check uses the interface counter**; VLAN blocking bits 6:4 (TX) and 2:0 (RX) |
| 0x02 | 0x1a | u8 | disable_pkey_check |
| 0x03 | 0x1b | u8 | pkey_index |
| 0x04 | 0x1c | u8 | **counter_index** |
| 0x05 | 0x1d | u8 | grh_mylmc (for Ethernet UD: source MAC table index) |
| 0x06 | 0x1e | be16 | rlid |
| 0x08 | 0x20 | u8 | **ackto**: bits 2:0 link type — **1 = Ethernet**; bits 7:3 ack timeout |
| 0x09 | 0x21 | u8 | mgid_index |
| 0x0a | 0x22 | u8 | static_rate |
| 0x0b | 0x23 | u8 | hop_limit |
| 0x0c | 0x24 | be32 | tclass_flowlabel |
| 0x10 | 0x28 | 16 bytes | rgid |
| 0x20 | 0x38 | u8 | **sched_queue**: bit 6 = port − 1; bits 5:3 = user priority; Ethernet default **0x83** (port 1) / **0xC3** (port 2) |
| 0x21 | 0x39 | u8 | vlan_index |
| 0x22 | 0x3a | u8 | feup: bit 6 force Ethernet priority, bit 5 force source MAC, bit 3 force VLAN |
| 0x23 | 0x3b | u8 | fvl_rx |
| 0x26 | 0x3e | 6 bytes | dmac |

#### 4.8.2 QP buffer layout

One buffer holds both queues, described by one run of MTT entries (context
0xc8–0xcf). **The queue with the larger stride comes first**; if the receive
stride is larger than the send stride, the RQ is at offset 0 and the SQ
follows it; otherwise (equal or smaller) the SQ is at offset 0 and the RQ
follows the SQ.

- SQ bytes = 2^(log SQ size) × 64 (stride 64 = one TXBB).
- RQ bytes = 2^(log RQ size) × RQ stride.

For the RX QP with a 16-byte receive stride: the SQ (one 64-byte TXBB) is at
offset 0 and the RQ at offset 64. `mlx4_en` writes 0x80000000 (owner bit set,
see 5.7) into the first dword of that unused TXBB so the device never treats
it as a send WQE. Buffer size = 64 + N × 16, rounded up to whole pages.

For the TX QP: rq_size_stride = 0 (one 16-byte RQ entry), so the SQ is at
offset 0 and the phantom RQ would sit right after the SQ. `mlx4_en` allocates
only the SQ (rounded up to 4 KiB) and never posts receives on it. A cautious
driver may allocate one extra page; **[HW-CHECK]** not needed.

#### 4.8.3 State transitions

A QP is created by moving it from RESET (the state every QP number is in after
INIT_HCA) through INIT and RTR to RTS with three commands. There is no
separate "create" command; the QPN is chosen by the driver (4.1).

Modify-QP input mailbox (0x100 bytes, zeroed):

| Offset | Width | Field |
|---|---|---|
| 0x00 | be32 | optional-parameter mask (0 for the transitions here) |
| 0x04 | 4 bytes | reserved |
| 0x08 | 0xF8 | QP context (4.8.1) |

in_modifier = QPN (bit 31 = SQD event request, 0), op_mod = 0.

| From → to | Command | Notes |
|---|---|---|
| RST → INIT | RST2INIT_QP 0x019 | context flags state = 1; MTT fields (0xc8, 0xcb, 0xcc) MUST be filled in this step |
| INIT → RTR | INIT2RTR_QP 0x01a | context flags state = 2; for the TX QP set params2 FPP here if BMME bit 24 (PORT_REMAP) |
| RTR → RTS | RTR2RTS_QP 0x01b | context flags state = 3 |
| any → RST | 2RST_QP 0x021 | **op_mod 2**, in_param 0, no mailbox |
| any → ERR | 2ERR_QP 0x01e | not needed |

`mlx4_en` sends the **same context** in all three steps, changing only the
state nibble (bits 31:28 of flags) and clearing params2 FPP except for the
INIT→RTR step. The optional-parameter mask is 0 each time.

QUERY_QP (0x022, in_modifier QPN, output mailbox) returns the same layout, the
context at mailbox offset 0x08. Useful for debugging.

Optional-parameter mask bits (not used here): 0 alternate path, 1 RRE, 2 RAE,
3 RWE, 4 P_Key index, 5 Q_Key, 6 RNR timeout, 7 primary path, 8 SRA max,
9 RRA max, 10 PM state, 12 retry count, 13 RNR retry, 14 ack timeout,
16 scheduling queue, 20 counter index, 21 VLAN stripping.

#### 4.8.4 Context values for the two QPs

All other fields 0.

| Field | RX QP | TX QP |
|---|---|---|
| flags | (state << 28) \| 0x00070000 | same |
| pd | PD (4.1) | PD |
| mtu_msgmax | 0xff | 0xff |
| rq_size_stride | (log2 N << 3) \| (log2 stride − 4); 16-byte stride → (log2 N << 3) | 0 |
| sq_size_stride | 0x02 (one 64-byte TXBB) | (log2 M << 3) \| 0x02 |
| usr_page | driver UAR index | driver UAR index |
| local_qpn | RX QPN | TX QPN |
| pri_path.ackto | 0x01 | 0x01 |
| pri_path.sched_queue | 0x83 \| ((port − 1) << 6) | same |
| pri_path.counter_index | counter (below) | counter |
| pri_path.control / fl | see 5.10 (loopback) | 0 |
| cqn_send | RX CQN (unused) | TX CQN |
| cqn_recv | RX CQN | TX CQN (unused) |
| db_rec_addr | physical address of RX doorbell record | physical address of a zeroed 8-byte record (unused) |
| param3 | 0x40000000 (keep VLAN tags in frames; add 0x20000000 only if you want the FCS kept and capability bit 34 is set) | 0x40000000 |
| params2 | 0 | FPP 0x00000008 only in the INIT→RTR command, only if BMME bit 24 |
| log_page_size, mtt_base_addr_h/l | the QP buffer's MTT run | same |

Counter index: if counters were enabled in INIT_HCA (capability bit 48),
use port − 1 (what Linux allocates first). If not, Linux ends up writing 0xFF.
**[HW-CHECK]** either choice.

`mlx4_en` sets param3 bit 30 only when VLAN receive offload is off; for a UEFI
driver that must hand the stack complete frames, set it (the device then
leaves VLAN tags in the frame and does not report them in the CQE).

#### 4.8.5 Receive doorbell

The receive queue has **no MMIO doorbell**: the device reads the receive
doorbell record (4.4). After writing new receive WQEs, issue a write barrier,
then store the new producer counter (bits 15:0, be32) into the record.

#### 4.8.6 Send doorbell

After writing a send WQE and setting its owner bit (5.7), issue a write
barrier and write the 32-bit value **QPN << 8** in big-endian byte order to
offset 0x14 of the UAR page named in the QP's `usr_page`. (On a little-endian
CPU the 32-bit MMIO store value is the byte-swapped form of QPN << 8.)

#### 4.8.7 Teardown

2RST_QP on each QP (op_mod 2, in_param 0, in_modifier QPN). After it, the QP
number is back in RESET and its buffers may be freed.

Sources: `include/linux/mlx4/qp.h` — `mlx4_qp_context`, `mlx4_qp_path`, `MLX4_QP_STATE_*`, `MLX4_QP_ST_*`, `MLX4_QP_OPTPAR_*`, `MLX4_QP_BIT_*`, `MLX4_FL_*`, `MLX4_CTRL_*`, `MLX4_STRIP_VLAN`; `qp.c` — `__mlx4_qp_modify` (opcode table, mailbox format, 2RST), `mlx4_qp_to_ready`, `mlx4_qp_query`; `en_resources.c` — `mlx4_en_fill_qp_context`; `en_rx.c` — `mlx4_en_config_rss_qp`, `mlx4_en_activate_rx_rings`, `mlx4_en_config_rss_steer` (single-ring path); `en_tx.c` — `mlx4_en_create_tx_ring`, `mlx4_en_activate_tx_ring`, `mlx4_en_xmit_doorbell`; FreeBSD `mlx4_ib/mlx4_ib_qp.c` — `set_kernel_sq_size` (queue order), `__mlx4_ib_modify_qp` (raw-packet MTU/msgmax, link type, VSD); `resource_tracker.c` — VSD handling (bit 30 meaning).

---

## 5. The Ethernet data path

### 5.1 Port type

QUERY_PORT byte 0 bits 1:0 give the supported types (bit 0 IB, bit 1
Ethernet). Linux's rule:

- Ethernet only (value 2): the port is Ethernet.
- IB only (value 1): not usable by this driver.
- Both (value 3, VPI): use the firmware's suggestion (byte 0 bit 3: 1 =
  Ethernet) unless configured otherwise. If port sensing is allowed
  (capability DPDP bit 12 and SENSE_SUPPORT bit 55) and QUERY_PORT's
  default-sense bit (byte 0 bit 4) is set, Linux issues SENSE_PORT
  (in_modifier port; immediate output 1 = IB, 2 = Ethernet, 0 = none; values
  > 2 are invalid) and uses the result.

The protocol is not programmed by a command: the port behaves as Ethernet
when driven with the Ethernet forms of SET_PORT (op_mod 1) and Ethernet QPs.
For a VPI card whose firmware defaults a port to InfiniBand, the port's link
type is a firmware (NVRAM) setting outside this document's scope
**[HW-CHECK]** on the target blades.

Sources: `main.c` — `mlx4_dev_cap` (port type choice), `mlx4_change_port_types`; `sense.c` — `mlx4_SENSE_PORT`; `fw.c` — `mlx4_QUERY_PORT`.

### 5.2 MAC address

The port's MAC is the low 48 bits of QUERY_PORT be64 at 0x10 (3.5). If it is
all zeros Linux substitutes a random locally administered address. The MAC is
also what the driver registers in the port MAC table and steering (5.3, 5.4).

Sources: `fw.c` — `mlx4_QUERY_PORT`, `mlx4_replace_zero_macs`; `en_netdev.c` — `mlx4_en_init_netdev`.

### 5.3 SET_PORT (Ethernet)

SET_PORT, opcode 0x00c, **op_mod 1** (Ethernet), in_modifier = (sub-operation
<< 8) | port, input mailbox.

| Sub-operation | Value | Section |
|---|---|---|
| GENERAL | 0x0 | below |
| RQP_CALC | 0x1 | below (A0 steering only) |
| MAC_TABLE | 0x2 | below |
| VLAN_TABLE | 0x3 | not needed |
| PRIO_MAP | 0x4 | not needed |

**GENERAL** (mailbox bytes; zero the rest):

| Offset | Width | Field | Value |
|---|---|---|---|
| 0x02 | u8 | flags2 | bit 5 = user MTU valid, bit 6 = user MAC valid, bit 1 = ignore-FCS valid (0 in the main call) |
| 0x03 | u8 | flags | bit 0 MTU valid, bit 1 RX pause valid, bit 2 TX pause valid → **0x07** |
| 0x04 | u8 | ignore_fcs / roce_mode | 0 |
| 0x06 | be16 | mtu | maximum frame size the port accepts, see 5.9 (1526 for a 1500-byte MTU) |
| 0x08 | u8 | pptx | bit 7 = transmit global pause (Linux default **0x80**) |
| 0x09 | u8 | pfctx | per-priority pause mask (0) |
| 0x0c | u8 | pprx | bit 7 = receive global pause (Linux default **0x80**) |
| 0x0d | u8 | pfcrx | 0 |
| 0x14 | u8 | phv_en | 0 |
| 0x1a | be16 | user_mtu | (second call only, optional) the L3 MTU, with flags2 bit 5 |
| 0x1e | 6 bytes | user_mac | not needed |

Linux follows the GENERAL call with a second GENERAL call carrying only
flags2 = 0x20 and user_mtu = the L3 MTU (informational for firmware).
Optional.

**MAC_TABLE**: the mailbox holds the whole table, 128 entries × be64 (1024
bytes). Entry = bit 63 valid | MAC in bits 47:0. Unused entries 0. Register
the port MAC at **index 0**. The command replaces the whole table.

**RQP_CALC** (A0 steering only, 5.4): mailbox:

| Offset | Width | Field | Value |
|---|---|---|---|
| 0x00 | be32 | base_qpn | the RX QPN (receives the frames for MAC index 0) |
| 0x05 | u8 | n_mac | log2 of MAC table entries used for steering (Linux: log_num_macs, normally 7) |
| 0x06 | u8 | n_vlan | 0 |
| 0x07 | u8 | n_prio | 0 |
| 0x0b | u8 | mac_miss | 0 |
| 0x0c | u8 | intra_no_vlan | 0 |
| 0x0d | u8 | no_vlan | 0 (VLAN index used for untagged frames) |
| 0x0e | u8 | intra_vlan_miss | 0 |
| 0x0f | u8 | vlan_miss | 1 (VLAN index used for unknown VLANs) |
| 0x13 | u8 | no_vlan_prio | 0 |
| 0x14 | be32 | promisc | bit 31 = unicast promiscuous; bits 23:0 = base_qpn |
| 0x18 | be32 | mcast | bits 31:30 = multicast mode (0 direct only, 1 direct, 2 default); bits 23:0 = base_qpn. Linux uses 1 when capability bit 42 is set, else 2 |

Sources: `port.c` — `mlx4_SET_PORT_general`, `mlx4_SET_PORT_user_mtu`, `mlx4_SET_PORT_qpn_calc`, `mlx4_set_port_mac_table`, `__mlx4_register_mac`, `MLX4_MAC_VALID`, `MLX4_FLAG*_V_*`; `mlx4.h` — `mlx4_set_port_general_context`, `mlx4_set_port_rqp_calc_context`, `SET_PORT_GEN_ALL_VALID`, `SET_PORT_*_SHIFT`, `MCAST_*`, `MLX4_MAX_MAC_NUM`; `include/linux/mlx4/cmd.h` — `MLX4_SET_PORT_*`; `en_main.c` — `mlx4_en_get_profile` (pause defaults).

### 5.4 Receive steering

The device decides which QP receives an incoming frame. Linux has three
modes; which one it uses on a ConnectX-3 depends on capabilities and a module
parameter:

- **B0** (Linux default on ConnectX-3): used when capability bits 41
  (VEP_UC_STEER) **and** 42 (VEP_MC_STEER) are both set. INIT_HCA byte 0xd8
  bit 3 = 1. Frames are steered through entries in the MCG table keyed by
  destination MAC.
- **A0** (older ConnectX): used when either bit is missing. INIT_HCA 0xd8
  bit 3 = 0. Frames are steered by the port MAC table plus SET_PORT RQP_CALC.
- Device-managed flow steering: only when explicitly requested; not covered.

**Recommended**: follow the same rule (B0 if both flags, else A0). In both
modes, first register the MAC in the MAC table (5.3).

#### 5.4.1 B0: MCG entries

A steering entry is a 16-byte "GID" plus a list of member QPs, stored in the
MCG table. For Ethernet the GID is built from a MAC:

| GID byte | Value |
|---|---|
| 0–4 | 0 |
| 5 | port number |
| 6 | 0 |
| 7 | steer type << 1: **unicast = 0x02**, multicast/broadcast = 0x00 |
| 8–9 | 0 |
| 10–15 | MAC, in transmission order |

MGM (MCG entry) layout, entry size 2^log_mgm_entry_size (1024 bytes with the
recommended profile):

| Offset | Width | Field |
|---|---|---|
| 0x00 | be32 | next_gid_index: (index of the next entry in the chain) << 6; 0 = end |
| 0x04 | be32 | members_count: bits 23:0 = number of member QPs; bits 31:30 = protocol (**1 = Ethernet**) |
| 0x08 | 8 bytes | reserved |
| 0x10 | 16 bytes | GID |
| 0x20 | be32 × n | member QPs: bits 23:0 QPN; bit 30 = block multicast loopback |

Capacity: 4 × (entry size / 16 − 2) QPs per entry (248 for 1024 bytes).

The MCG table has num_mcgs entries: the first half (MGM, indices 0 ..
num_mcgs/2 − 1) is a hash table; the second half (AMGM) holds overflow
entries chained from it.

**Attach a QP to a GID** (unicast MAC, or broadcast / multicast):

1. MGID_HASH (0x027): input mailbox = the 16-byte GID at offset 0; op_mod =
   **1 if capability bit 42 (VEP_MC_STEER)** is set, else 0; immediate output
   (low 16 bits) = hash index `h`.
2. READ_MCG (0x025) in_modifier `h`, output mailbox → entry E.
3. Walk the chain: if E.members_count bits 23:0 is 0, E is free (use it at
   index `h`); else if E.GID equals ours and E.protocol is 1, use it; else
   follow next_gid_index >> 6 (READ_MCG again) until it is 0.
4. If no entry was found, take a free AMGM index `a` (num_mcgs/2 ≤ a <
   num_mcgs), build a new entry there, and afterwards link it: READ_MCG the
   last entry of the chain, set its next_gid_index = a << 6, WRITE_MCG it back.
5. In the chosen entry: set the GID (if new), append the RX QPN (bit 30 = 0 —
   Linux does not block loopback for either the unicast or the broadcast
   attach), set members_count = count | (1 << 30), and WRITE_MCG (0x026,
   op_mod 0, in_modifier = index, input mailbox = the entry).

A driver that also keeps promiscuous QPs must add them to every new entry
(Linux does); with a single RX QP that already is a member, nothing more is
needed.

Linux attaches, after INIT_PORT: the port MAC as **unicast** (byte 7 = 0x02)
and the broadcast address ff:ff:ff:ff:ff:ff as **multicast** (byte 7 = 0x00),
both to the RX QP. Multicast addresses the stack enables (UEFI SNP receive
filters) are attached the same way as multicast.

**Promiscuous (B0)**: WRITE_MCG with **op_mod 1**, in_modifier = (port << 16) |
(steer << 1), steer = 1 for unicast promiscuous and 0 for multicast
promiscuous; input mailbox = an MGM entry whose member list is the
promiscuous QPs (the RX QPN) and members_count = count | (1 << 30); GID and
next index 0. To leave promiscuous mode write the same command with an empty
member list (members_count = 1 << 30). **[HW-CHECK]** the empty-list form
(Linux rewrites it with the remaining promiscuous QPs, which for this driver is
none).

**Detach**: remove the QPN from the entry's member list and WRITE_MCG it; when
the list becomes empty Linux also unlinks/clears the entry. A boot driver that
only tears down at exit may skip detaches because CLOSE_HCA discards the table.

#### 5.4.2 A0: MAC table and RQP_CALC

In A0 mode the device computes the receiving QP as `base_qpn + MAC-table
index` (with VLAN/priority bits when configured). Linux (single receive
ring) passes the RX QPN as `base_qpn`. Configure:

1. MAC_TABLE with the port MAC at index 0 (5.3).
2. RQP_CALC with base_qpn = RX QPN, n_mac = 7 (or QUERY_PORT's log max MACs if
   smaller), promisc bit 31 = 0 (or 1 for promiscuous), mcast mode as in 5.3.
   The RX QPN SHOULD be aligned to 2^n_mac (128) and no other QP should use
   QPNs RX QPN + 1 .. RX QPN + 127: that is why 4.1 puts the RX QP at
   base_sqpn + 128 and the TX QP at base_sqpn + 256.
3. Broadcast and multicast delivery is controlled by the RQP_CALC `mcast` mode
   and by SET_MCAST_FLTR (below); the MCG attach calls are no-ops in A0 for
   Ethernet.

**[HW-CHECK]**: A0 has not been the Linux path on ConnectX-3 for years; if the
card reports both steering flags, use B0.

#### 5.4.3 Port multicast filter: SET_MCAST_FLTR

Opcode 0x048, in_modifier = port, op_mod = mode, in_param = MAC (bits 47:0) |
(clear << 63):

| Mode (op_mod) | Meaning |
|---|---|
| 0 CONFIG | add MAC to the port multicast filter; with clear = 1 first flush the filter |
| 1 DISABLE | filter off: all multicast passes (subject to steering) |
| 2 ENABLE | filter on: only listed multicast passes |

Linux in normal mode: DISABLE; CONFIG(broadcast, clear = 1); CONFIG(each
multicast); ENABLE. In promiscuous or all-multicast mode: DISABLE. At stop:
CONFIG(0, clear = 1). **Recommended** for a boot driver: DISABLE (let MCG
steering decide), or the Linux normal sequence when the SNP multicast filter
is in use.

Sources: `main.c` — `choose_steering_mode`; `mcg.c` — `mlx4_GID_HASH`, `mlx4_READ_ENTRY`, `mlx4_WRITE_ENTRY`, `mlx4_WRITE_PROMISC`, `find_entry`, `mlx4_qp_attach_common`, `mlx4_qp_detach_common`, `new_steering_entry`, `add_promisc_qp`, `remove_promisc_qp`, `mlx4_unicast_attach`, `mlx4_multicast_attach`, `mlx4_get_qp_per_mgm`; `mlx4.h` — `mlx4_mgm`, `MGM_QPN_MASK`, `MGM_BLCK_LB_BIT`; `include/linux/mlx4/device.h` — `MLX4_UC_STEER`, `MLX4_MC_STEER`, `MLX4_PROT_ETH`; `en_netdev.c` — `mlx4_en_uc_steer_add`, `mlx4_en_get_qp`, `mlx4_en_start_port`, `mlx4_en_set_promisc_mode`, `mlx4_en_clear_promisc_mode`, `mlx4_en_do_multicast`, `mlx4_en_stop_port`; `port.c` — `mlx4_SET_MCAST_FLTR`, `mlx4_get_base_qpn`; `mlx4.h` — `MLX4_MCAST_*`.

### 5.5 INIT_PORT and CLOSE_PORT

- **INIT_PORT** (0x009): in_param 0, in_modifier = port, op_mod 0, no mailbox
  (command interface revision 3). This brings the port up (link training
  starts). Linux issues it **after** the QPs are in RTS and after SET_PORT
  GENERAL (and RQP_CALC in A0), and attaches B0 steering entries after it.
- With interface revision 2 INIT_PORT takes an IB-style mailbox (flags at
  0x00 with VL cap << 4 and port width << 8, MTU at 0x04, max GIDs at 0x06,
  max P_Keys at 0x0a). Not needed for ConnectX-3 **[HW-CHECK]** revision 3.
- **CLOSE_PORT** (0x00a): in_modifier = port. Linux issues it first when
  stopping a port, before destroying QPs.

Sources: `fw.c` — `mlx4_INIT_PORT`, `mlx4_CLOSE_PORT`; `en_netdev.c` — `mlx4_en_start_port`, `mlx4_en_stop_port`.

### 5.6 Receive path

**Descriptor (receive WQE)**: an array of 16-byte data segments; the RQ
stride is the WQE size, a power of two ≥ 16. `mlx4_en` with a 1500-byte MTU
uses **one segment, stride 16**.

Data segment:

| Offset | Width | Field |
|---|---|---|
| 0x0 | be32 | byte_count: buffer length in bytes (bit 31 must be 0) |
| 0x4 | be32 | lkey: the region's L_Key (4.3) |
| 0x8 | be64 | addr: physical address of the buffer |

If the stride has room for more segments than used, fill the unused ones with
byte_count 0, lkey 0x00000100 (the padding memory type), addr 0.

**Buffer size**: at least the largest frame accepted: MTU + 14 (header) + 8
(two VLAN tags) = 1522 for MTU 1500; the FCS is removed unless param3 bit 29
is set (then +4). `mlx4_en` uses one buffer of that effective size per
descriptor (fragments of at most 2048 bytes). **Recommended**: 2048-byte
buffers, one per descriptor.

**Posting**: keep a free-running producer counter `prod`. Write descriptor
at RQ slot (prod mod N), increment prod; after a batch, write barrier, then
store `prod mod 2^16` into the RX doorbell record (4.8.5). `mlx4_en` fills the
whole ring (N descriptors) at start.

**Completion**: poll the RX CQ (4.7). Receive completions arrive in RQ
order, one per descriptor, so the descriptor for a CQE is slot (RX
consumer counter mod N); CQE `wqe_index` also gives it. For each valid CQE:

- If opcode (byte 0x1f bits 4:0) is **0x1e**: error CQE (syndrome at 0x1b);
  drop and repost the buffer.
- If byte 0x13 bit 4 (bad FCS) is set: drop.
- Otherwise frame length = be32 at 0x14 (minus 4 if FCS was kept). The frame
  is in the descriptor's buffer from offset 0.
- Update the CQ consumer index record (4.7) and repost a buffer in the freed
  RQ slot, then update the RX doorbell record.

Sources: `en_rx.c` — `mlx4_en_calc_rx_buf`, `mlx4_en_init_rx_desc`, `mlx4_en_activate_rx_rings`, `mlx4_en_update_rx_prod_db`, `mlx4_en_process_rx_cq`, `mlx4_en_config_rss_qp`; `mlx4_en.h` — `mlx4_en_rx_desc`, `MLX4_EN_MEMTYPE_PAD`, `MLX4_EN_EFF_MTU`, `DS_SIZE`; `include/linux/mlx4/qp.h` — `mlx4_wqe_data_seg`.

### 5.7 Send path

The send queue is an array of 64-byte **TXBBs** (send basic blocks); a send
WQE occupies one or more consecutive TXBBs (size rounded up to 64). A
free-running counter `prod` counts TXBBs posted.

**Send WQE** for one frame from one buffer (32 bytes, one TXBB):

Control segment (16 bytes, WQE offset 0x00):

| Offset | Width | Field |
|---|---|---|
| 0x00 | be32 | owner_opcode: bit 31 **owner** (below); bits 4:0 **opcode** = 0x0a SEND; bit 28/27 inner IP/L4 checksum (tunnels, 0); bit 29 NEC (0) |
| 0x04 | be16 | vlan_tag: VLAN TCI to insert (0 = none) |
| 0x06 | u8 | ins_vlan: bit 6 insert C-VLAN, bit 7 insert S-VLAN (0) |
| 0x07 | u8 | fence_size: bits 5:0 = **WQE size in 16-byte units** including this segment (2 for control + one data segment); bit 6 fence |
| 0x08 | be32 | srcrb_flags: bits 3:2 = **3 (generate a CQE)**; bit 1 solicited event; bit 4 IP checksum; bit 5 TCP/UDP checksum; bit 0 force loopback; bit 7 strong ordering. `mlx4_en` always sets CQ-update and solicited: **0x0000000E** |
| 0x0c | be32 | imm (0) |

Data segment (16 bytes, WQE offset 0x10): as in 5.6 — byte_count = frame
length, lkey, physical address of the frame.

**Inline alternative** (small frames, no separate buffer): after the control
segment, an inline segment: be32 = 0x80000000 | length, followed by the frame
bytes. An inline segment must not cross a 64-byte boundary of the WQE: the
first one holds at most 64 − 16 − 4 = 44 bytes; continue with another inline
header at the next 64-byte boundary. Frames shorter than 17 bytes are padded
to 17. Size in 16-byte units counts the headers and data, rounded up.

**Owner bit**: a TXBB at counter value `p` belongs to the device when its
owner bit equals ((p / M) mod 2), M = SQ size in TXBBs. So the WQE written at
counter `prod` gets owner = 1 if (prod & M) ≠ 0, else 0.

**Initialise the SQ**: before first use, write 0xFFFFFFFF into the first dword
of every 64-byte TXBB (owner = 1, invalid opcode: not the device's in pass 0).

**Posting order** (matters: the device may prefetch):

1. Write the data segment(s): address and lkey first, then (after a write
   barrier) byte_count last.
2. Write control-segment bytes 0x04–0x0f.
3. Write barrier; then write owner_opcode (dword 0x00) with the owner bit.
4. `prod += TXBBs used`.
5. Write barrier; send doorbell: be32 (TX QPN << 8) to UAR + 0x14 (4.8.6).
   Several WQEs may be posted before one doorbell.

**Headroom**: the device prefetches up to 2 KiB beyond the current position.
Keep at least 33 TXBBs (2048/64 + 1) unused: outstanding TXBBs (prod −
completed) ≤ M − 33 − (max TXBBs per WQE). With M = 256 and single-TXBB WQEs,
at most 222 outstanding.

**Completion**: poll the TX CQ. Each WQE posted with CQ-update generates one
CQE; `wqe_index` = TXBB index (mod 2^16, masked to the ring) of that WQE.
Error CQEs (opcode 0x1e) mean the QP moved to an error state: Linux restarts
the port (all QPs to RESET and back); a boot driver should do the same or
fail the device. After consuming, update the TX CQ consumer index record.

**Stamping freed TXBBs**: after a WQE completes, Linux overwrites the first
dword of each 64-byte block of it with 0x7FFFFFFF | (o << 31), where o is the
owner value that WQE was posted with. This keeps a stale descriptor from being
taken as valid on the next pass through the ring (it matters for WQEs longer
than one TXBB). **Recommended**: do the same.

**Short frames**: Linux does not pad non-inline frames shorter than 60 bytes;
the device pads them on the wire (inferred). Padding to 60 bytes in software
is harmless.

Sources: `en_tx.c` — `mlx4_en_xmit`, `mlx4_en_xmit_frame`, `build_inline_wqe`, `mlx4_en_build_dma_wqe`, `mlx4_en_tx_write_desc`, `mlx4_en_xmit_doorbell`, `mlx4_en_process_tx_cq`, `mlx4_en_stamp_wqe`, `mlx4_en_activate_tx_ring`, `mlx4_en_is_tx_ring_full`; `mlx4_en.h` — `TXBB_SIZE`, `HEADROOM`, `STAMP_*`, `MLX4_EN_BIT_DESC_OWN`, `MIN_PKT_LEN`, `mlx4_en_tx_desc`; `en_netdev.c` — `mlx4_en_init_netdev` (`ctrl_flags`), `mlx4_en_start_port` (0xFFFFFFFF init); `include/linux/mlx4/qp.h` — `mlx4_wqe_ctrl_seg`, `mlx4_wqe_data_seg`, `mlx4_wqe_inline_seg`, `MLX4_WQE_CTRL_*`, `MLX4_INLINE_*`; `include/linux/mlx4/device.h` — `MLX4_OPCODE_SEND`.

### 5.8 Link state and speed

- **Poll**: QUERY_PORT (3.5) byte 0 bit 7 = link up; byte 5 & 0x6f = speed
  code. It is a firmware command (tens of microseconds to milliseconds), fine
  for SNP `GetStatus` but not for every receive poll.
- **Event**: with port change (0x09) mapped to the EQ (4.6), polling the EQ
  gives subtype 4 (active) or 1 (down) with the port in bits 31:28 of the be32
  at EQE 0x0c.
- After INIT_PORT the link takes time to come up (autonegotiation, typically
  seconds; the source gives no bound). A UEFI driver should report "no media"
  until it does.

Sources: `en_port.c` — `mlx4_en_QUERY_PORT`; `en_port.h` — `MLX4_EN_LINK_UP_MASK`, speed codes; `eq.c` — `mlx4_eq_int` (port change handling); `en_netdev.c` — `mlx4_en_linkstate`.

### 5.9 MTU

- Port maximum: QUERY_PORT be16 at 0x02 (Linux uses it as the maximum MTU).
- SET_PORT GENERAL `mtu` = MTU + 14 + 8 + 4 (Ethernet header, two VLAN tags,
  FCS): **1526** for MTU 1500.
- Receive buffers: MTU + 22 bytes minimum (5.6).
- SNP: MaxPacketSize 1500, MediaHeaderSize 14.

Sources: `en_netdev.c` — `mlx4_en_start_port` (SET_PORT general argument), `mlx4_en_init_netdev` (`max_mtu`); `en_rx.c` — `mlx4_en_calc_rx_buf`; `mlx4_en.h` — `MLX4_EN_EFF_MTU`.

### 5.10 Loopback of the driver's own frames

The adapter's embedded switch can deliver a frame sent on a port back to
receive QPs of the same port (multicast and broadcast in particular). Linux
prevents the RX QP from receiving its own multicast by setting, in the RX QP's
primary path, `control` bit 7 (source check by interface counter) and `fl`
bit 1 (multicast loopback source check), but only when capability ext-2 bit
19 (LB_SRC_CHK) is set and a real (non-sink) counter is in use; it also drops,
in software, received multicast frames whose source MAC is one of its own.
**Recommended**: do both where available; otherwise drop received frames whose
source MAC equals the port MAC. **[HW-CHECK]** whether loopback actually
happens without SR-IOV.

Sources: `en_resources.c` — `mlx4_en_fill_qp_context`; `en_rx.c` — `mlx4_en_process_rx_cq` (RX_FILTER_NEEDED); `include/linux/mlx4/qp.h` — `MLX4_FL_ETH_SRC_CHECK_MC_LB`, `MLX4_CTRL_ETH_SRC_CHECK_IF_COUNTER`.

---

## 6. Minimal polling driver: walkthrough

The ordered list of actions from PCI probe to one frame each way, then
shutdown. Numbers in brackets refer to sections.

### 6.1 Start (Driver Binding `Start`)

1. Open `EFI_PCI_IO_PROTOCOL`; check vendor 0x15b3, device 0x1003 or 0x1007.
   Enable memory space, bus master, 64-bit DMA; set INTx disable [1.7].
2. Read the ownership semaphore; stop if not 0 [1.5].
3. Reset the device; restore config space; re-enable memory space and bus
   master if the restore did not [1.6].
4. Allocate one 4 KiB mailbox (DMA common buffer); set `toggle` [2.4].
5. QUERY_FW; record FW area size, catastrophic buffer location; require
   interface revision 3 (or handle 2) [3.1].
6. Allocate the FW area; MAP_FA; RUN_FW [3.2].
7. MOD_STAT_CFG (4 KiB pages) [3.3].
8. QUERY_DEV_CAP; check min page ≤ 4 KiB, UAR area ≤ BAR 2 size and > 128
   pages, ports ≤ 2 [3.4].
9. QUERY_PORT for the chosen port: supported types include Ethernet; MAC;
   MTU cap; log max MACs [3.5, 5.1, 5.2].
10. Decide B0 or A0 steering from capability bits 41/42 [5.4].
11. Compute the profile and ICM layout; check against max ICM size [3.7].
12. SET_ICM_SIZE; allocate aux pages; MAP_ICM_AUX [3.8].
13. Allocate and MAP_ICM every table range [3.9].
14. INIT_HCA [3.10]; if SYS_EQS, QUERY_FUNC [3.11].
15. Choose object numbers: PD, UAR page, MPT index, EQN, CQNs, QPNs, MTT runs
    [4.1].
16. SW2HW_MPT for the physical region; compute the L_Key [4.3].
17. EQ: allocate buffer (256 × 32 bytes), mark all entries hardware-owned,
    write its MTT entries, SW2HW_EQ, MAP_EQ with the recommended mask [4.6].
18. CONF_SPECIAL_QP(base_sqpn) [3.12].
19. CQs: allocate buffers and 8-byte doorbell records, fill buffers with 0xCC,
    write MTT entries, SW2HW_CQ for RX and TX [4.7].
20. RX QP: allocate the buffer (64 + N × 16 bytes, page-rounded), write
    0x80000000 into its first dword, allocate the doorbell record (0), write
    MTT entries; fill all N receive descriptors with 2 KiB buffers; RST2INIT,
    INIT2RTR, RTR2RTS with the RX context [4.8.4, 5.6]; then store N into the
    doorbell record.
21. TX QP: allocate the SQ buffer (M × 64 bytes), write 0xFFFFFFFF into the
    first dword of each TXBB, write MTT entries; RST2INIT, INIT2RTR (with FPP
    if PORT_REMAP), RTR2RTS with the TX context [4.8.4, 5.7].
22. SET_PORT MAC_TABLE with the port MAC at index 0 [5.3].
23. SET_PORT GENERAL (MTU 1526, pause 0x80/0x80) [5.3]; optionally the
    user-MTU call.
24. A0 only: SET_PORT RQP_CALC [5.4.2].
25. INIT_PORT [5.5].
26. B0 only: attach unicast(port MAC) and multicast(broadcast) to the RX QP
    [5.4.1].
27. SET_MCAST_FLTR DISABLE (or the Linux normal sequence) [5.4.3].
28. Install `EFI_SIMPLE_NETWORK_PROTOCOL` on a child handle with a MAC device
    path (issue #4); report media absent until QUERY_PORT shows link up.

The Linux order differs from this only in that `mlx4_en` registers the MAC
(step 22) before creating its QPs and activates CQs before QPs; both orders
have the QPs in RTS before INIT_PORT.

### 6.2 Send one frame (SNP `Transmit`)

1. Reclaim completed TXBBs by polling the TX CQ (5.7).
2. If outstanding TXBBs would exceed M − 33 − 1, return "not ready".
3. Copy or map the frame (DMA), build the 2-segment WQE at slot (prod mod M)
   in the order of 5.7, advance prod, ring the send doorbell.
4. SNP `GetStatus` later returns the buffer when its CQE arrives.

### 6.3 Receive one frame (SNP `Receive`)

1. Poll the RX CQ; if no valid CQE, return "not ready".
2. Check opcode/error/FCS; copy `byte_cnt` bytes from the slot's buffer to the
   caller.
3. Update the RX CQ consumer record; repost the slot's buffer; update the RX
   doorbell record.

Also poll the EQ from time to time (port change, error events) and write its
consumer index doorbell.

### 6.4 Stop (Driver Binding `Stop`) and ExitBootServices

UEFI requires that at ExitBootServices the device stops all DMA into memory
the OS will take over. Everything the device may touch — FW area, aux pages,
ICM, mailboxes, queues, receive buffers — is boot-services memory. So the
teardown MUST end with firmware no longer using host memory.

Orderly teardown (both `Stop` and the ExitBootServices event; in the event
handler do not free memory or call memory services):

1. CLOSE_PORT [5.5].
2. 2RST_QP for the TX and RX QPs [4.8.7].
3. HW2SW_CQ for TX and RX CQs [4.7].
4. HW2SW_MPT [4.3].
5. CONF_SPECIAL_QP(0) [3.12].
6. MAP_EQ with the unmap bit; HW2SW_EQ [4.6].
7. CLOSE_HCA [3.13].
8. UNMAP_ICM for every mapped range; UNMAP_ICM_AUX; UNMAP_FA [3.13].
9. Release ownership: write 0 to the semaphore (skip the 1 s wait in the
   ExitBootServices handler; the OS driver resets the device on load).
10. Clear Bus Master Enable in the PCI command register.
11. `Stop` only: free all memory.

If any command fails or times out: reset the device (1.6; it takes about 1 s)
and clear Bus Master Enable. After a reset the firmware is stopped and does
not use the FW area. **[HW-CHECK]** that clearing Bus Master Enable alone,
without the command sequence, leaves the device in a state the OS driver
recovers from (the OS driver resets it, so it should).

Sources: `main.c` — `mlx4_load_one`, `mlx4_unload_one`; `en_netdev.c` — `mlx4_en_start_port`, `mlx4_en_stop_port`; `en_main.c` — `mlx4_en_probe`, `mlx4_en_remove`.

---

## 7. Hardware verification checklist

The sources do not settle these; each needs a test on a ConnectX-3 (and one on
a ConnectX-3 Pro if available). The driver should log enough to answer them
(QUERY_FW version and revision, QUERY_DEV_CAP raw bytes 0x10–0xa7, each
command's status).

1. **Toggle bit after reset** (2.4): the T bit reads 0 after reset, so
   `toggle` starts at 1. Confirm; use the "read T, set toggle = NOT T" rule.
2. **Hardware semaphore and ownership semantics** (1.5, 1.6): reading 0 means
   acquired; no explicit release of the reset semaphore.
3. **Command interface revision** (3.1): expected 3 on ConnectX-3.
4. **Small profile accepted** (3.7): INIT_HCA with a few-hundred-QP profile.
   Also the EQ slot sizing difference from Linux.
5. **EQ and CQ ownership initialisation** (4.6, 4.7): the driver pre-marks
   entries hardware-owned; confirm nothing spurious is seen right after
   SW2HW_EQ/SW2HW_CQ, and read the SW_CQ_INIT flag on the real card.
6. **32-byte CQEs/EQEs**: Linux enables 64-byte entries on ConnectX-3 when
   offered; this spec leaves them off (Linux's "disabled" path). Confirm CQEs
   are 32 bytes with INIT_HCA 0x58 = 0.
7. **Steering mode** (5.4): expect both VEP flags set (B0). Confirm unicast
   and broadcast reception through MCG entries; confirm the promiscuous
   write format including the empty list.
8. **CONF_SPECIAL_QP necessity** (3.12).
9. **Counter index** (4.8.4) with and without INIT_HCA counters enabled.
10. **TX QP phantom RQ** (4.8.2): no fault with rq_size_stride = 0 and no
    extra buffer.
11. **MTT writes**: direct ICM writes are seen by the device (they are on
    Linux); WRITE_MTT availability natively.
12. **Port type on the X9 blades' ConnectX-3** (5.1): supported types and
    suggested type from QUERY_PORT.
13. **Link-up time** after INIT_PORT (5.8).
14. **Own-frame loopback** (5.10).
15. **ExitBootServices**: the orderly teardown completes quickly, the OS
    driver then loads cleanly; also the reset-only fallback.
16. **L_Key collision with 0x100** (4.3): reserved MPT count on the card.

---

## Appendix A. Command opcodes

From `include/linux/mlx4/cmd.h`. Commands marked • are used in this document.

| Opcode | Name | | Opcode | Name |
|---|---|---|---|---|
| 0x001 | SYS_EN | | 0x02e | ACCESS_DDR / ACCESS_MEM |
| 0x002 | SYS_DIS | | 0x02f | SYNC_TPT |
| 0x003 | QUERY_DEV_CAP • | | 0x030 | DIAG_RPRT |
| 0x004 | QUERY_FW • | | 0x031 | NOP • |
| 0x005 | QUERY_DDR | | 0x032 | SUSPEND_QP |
| 0x006 | QUERY_ADAPTER • | | 0x033 | UNSUSPEND_QP |
| 0x007 | INIT_HCA • | | 0x034 | MOD_STAT_CFG • |
| 0x008 | CLOSE_HCA • | | 0x035 | SW2HW_SRQ |
| 0x009 | INIT_PORT • | | 0x036 | HW2SW_SRQ |
| 0x00a | CLOSE_PORT • | | 0x037 | QUERY_SRQ |
| 0x00b | QUERY_HCA | | 0x038 | SQD2SQD_QP |
| 0x00c | SET_PORT • | | 0x03a | CONFIG_DEV |
| 0x00d | SW2HW_MPT • | | 0x03b | ACCESS_REG |
| 0x00e | QUERY_MPT | | 0x040 | ARM_SRQ |
| 0x00f | HW2SW_MPT • | | 0x043 | QUERY_PORT • |
| 0x010 | READ_MTT | | 0x047 | SET_VLAN_FLTR |
| 0x011 | WRITE_MTT | | 0x048 | SET_MCAST_FLTR • |
| 0x012 | MAP_EQ • | | 0x049 | DUMP_ETH_STATS |
| 0x013 | SW2HW_EQ • | | 0x04d | SENSE_PORT • |
| 0x014 | HW2SW_EQ • | | 0x050 | HW_HEALTH_CHECK |
| 0x015 | QUERY_EQ | | 0x052 | SET_VEP |
| 0x016 | SW2HW_CQ • | | 0x054 | QUERY_IF_STAT |
| 0x017 | HW2SW_CQ • | | 0x055 | SET_IF_STAT |
| 0x018 | QUERY_CQ | | 0x056 | QUERY_FUNC • |
| 0x019 | RST2INIT_QP • | | 0x057 | ARM_COMM_CHANNEL |
| 0x01a | INIT2RTR_QP • | | 0x058 | GEN_EQE |
| 0x01b | RTR2RTS_QP • | | 0x059 | GET_OP_REQ |
| 0x01c | RTS2RTS_QP | | 0x05a | SET_NODE |
| 0x01d | SQERR2RTS_QP | | 0x05b | INFORM_FLR_DONE |
| 0x01e | 2ERR_QP | | 0x05c | VIRT_PORT_MAP |
| 0x01f | RTS2SQD_QP | | 0x061 | UPDATE_QP |
| 0x020 | SQD2RTS_QP | | 0x064 | FLOW_STEERING_IB_UC_QP_RANGE |
| 0x021 | 2RST_QP • | | 0x065 | QP_FLOW_STEERING_ATTACH |
| 0x022 | QUERY_QP • | | 0x066 | QP_FLOW_STEERING_DETACH |
| 0x023 | CONF_SPECIAL_QP • | | 0x068 | CONGESTION_CTRL |
| 0x024 | MAD_IFC | | 0x080 | ALLOCATE_VPP |
| 0x025 | READ_MCG • | | 0x081 | SET_VPORT_QOS |
| 0x026 | WRITE_MCG • | | 0x203 | MAD_DEMUX |
| 0x027 | MGID_HASH • | | 0xff6 | RUN_FW • |
| 0x02a | QUERY_DEBUG_MSG | | 0xff7 | DISABLE_LAM |
| 0x02b | SET_DEBUG_MSG | | 0xff8 | ENABLE_LAM |
| 0x02c | MODIFY_CQ | | 0xff9 | UNMAP_ICM • |
| 0x02d | INIT2INIT_QP | | 0xffa | MAP_ICM • |
| | | | 0xffb | UNMAP_ICM_AUX • |
| | | | 0xffc | MAP_ICM_AUX • |
| | | | 0xffd | SET_ICM_SIZE • |
| | | | 0xffe | UNMAP_FA • |
| | | | 0xfff | MAP_FA • |

Opcodes 0xf00–0xf0b (ALLOC_RES, FREE_RES, MCAST_ATTACH, UCAST_ATTACH,
PROMISC, QUERY_FUNC_CAP, QP_ATTACH) are virtual commands of the Linux
multi-function layer, never sent to firmware by a native driver.

Sub-operation codes:

| Command | op_mod / in_modifier codes |
|---|---|
| SET_PORT op_mod | 0 IB, 1 Ethernet, 4 beacon |
| SET_PORT Ethernet in_modifier bits 15:8 | 0 GENERAL, 1 RQP_CALC, 2 MAC_TABLE, 3 VLAN_TABLE, 4 PRIO_MAP, 5 GID_TABLE, 8 PRIO2TC, 9 SCHEDULER, 0xB VXLAN, 0xD RoCE address |
| SET_MCAST_FLTR op_mod | 0 CONFIG, 1 DISABLE, 2 ENABLE |
| 2RST_QP op_mod | 2 |
| HW2SW_EQ / HW2SW_CQ / HW2SW_MPT op_mod | 1 (no output mailbox) |
| SW2HW_CQ op_mod | 1 when SW_CQ_INIT and the driver initialised CQEs, else 0 |
| WRITE_MCG op_mod | 0 entry, 1 promiscuous default entry |
| MGID_HASH op_mod | 1 for Ethernet when VEP_MC_STEER, else 0 |
| NOP in_modifier | 0x1f ("finish as soon as possible") |
| CLOSE_HCA op_mod | 0 normal, 1 panic |

## Appendix B. Command status codes

See 2.6.

## Appendix C. Constants and sizes

| Constant | Value | Where |
|---|---|---|
| HCR offset / size | BAR 0 + 0x80680 / 0x1c | 1.3 |
| Ownership register | BAR 0 + 0x8069c | 1.5 |
| Reset block / reset reg / semaphore | BAR 0 + 0xf0000 / +0x10 / +0x3fc | 1.6 |
| Reset wait / vendor-ID poll / semaphore timeout | 1000 ms / 2 s / 10 s | 1.6 |
| Ownership release wait | 1000 ms | 1.5 |
| HCR GO / E / T bits | 23 / 22 / 21 of dword 0x18 | 2.2 |
| op_modifier shift | 12 | 2.2 |
| Polling token | 0xffff | 2.3 |
| Command timeout | 60 000 ms | 2.7 |
| Mailbox size / alignment | 4096 / 4096 | 2.5 |
| Supported command interface revisions | 2..3 (3 = new port commands) | 3.1 |
| ICM page | 4096 | 0.3 |
| Linux ICM chunk size | 256 KiB | 3.9 |
| cMPT sub-table size | 2^24 entries per type; types QP 0, SRQ 1, CQ 2, EQ 3 | 3.7 |
| INIT_HCA mailbox size / version | 0x200 / 2 | 3.10 |
| MOD_STAT_CFG size | 0x100 | 3.3 |
| Reserved UAR pages for EQ doorbells | 128 (4 KiB pages) | 1.4 |
| UAR send doorbell / CQ doorbell / EQ doorbell | +0x14 / +0x20 / +0x800 + 8 × (eqn mod 4) in page eqn/4 | 1.4 |
| MTT present bit | bit 0 | 4.2 |
| Invalid / padding L_Key | 0x00000100 | 4.3, 5.6 |
| MPT flags for a physical local-RW region | 0xF0020F00 | 4.3 |
| MPT pd_flags EN_INV | 0x03000000 | 4.3 |
| EQ entry size | 32 (64 when enabled) | 4.6 |
| EQ spare entries / CI update interval | 0x80 | 4.6 |
| EQ context flags (OK, armed) | 0x00000900 | 4.6 |
| CQ entry size | 32 (64 when enabled) | 4.7 |
| CQ initialisation byte | 0xCC | 4.7 |
| CQE owner / is-send / opcode masks | 0x80 / 0x40 / 0x1f of byte 0x1f | 4.7 |
| CQE error opcode | 0x1e | 4.7 |
| QP service type (raw Ethernet) | 0x7 (MLX) | 4.8 |
| QP mtu_msgmax | 0xff | 4.8.4 |
| sched_queue | 0x83 \| ((port − 1) << 6) | 4.8.4 |
| Link type (ackto bits 2:0) | 1 = Ethernet | 4.8.4 |
| param3 VLAN-strip disable / keep FCS | bit 30 / bit 29 | 4.8.1 |
| params2 FPP | bit 3 | 4.8.1 |
| RSS flag in QP flags | bit 13 | 4.8.1 |
| TXBB size | 64 | 5.7 |
| Send opcode | 0x0a | 5.7 |
| Send ctrl flags (CQ update + solicited) | 0x0000000E | 5.7 |
| Owner bit in owner_opcode | 0x80000000 | 5.7 |
| Initial TXBB stamp / freed stamp | 0xFFFFFFFF / 0x7FFFFFFF \| owner << 31 | 5.7 |
| Send headroom | 33 TXBBs (2 KiB + 1) | 5.7 |
| Inline flag / inline alignment / min inline length | bit 31 of byte count / 64 / 17 | 5.7 |
| Receive data segment size | 16 | 5.6 |
| MAC table size | 128 entries × 8 bytes; valid bit 63 | 5.3 |
| SET_PORT GENERAL flags | 0x07 | 5.3 |
| Pause enable (pptx/pprx) | 0x80 | 5.3 |
| MGM entry size (default) | 1024 (log 10) | 5.4 |
| QPs per MGM | 4 × (entry/16 − 2) | 5.4 |
| GID steer byte | byte 7: unicast 0x02, multicast 0x00 | 5.4 |
| MGM protocol Ethernet | 1 (bits 31:30 of members_count) | 5.4 |
| next_gid_index shift | 6 | 5.4 |
| Frame overhead for SET_PORT mtu | +26 (14 + 8 + 4) | 5.9 |

CQE error syndromes (byte 0x1b of an error CQE): 0x01 local length, 0x02
local QP operation, 0x04 local protection, 0x05 work request flushed, 0x06
memory-window bind, 0x10 bad response, 0x11 local access, 0x12 remote invalid
request, 0x13 remote access, 0x14 remote operation, 0x15 transport retry
exceeded, 0x16 RNR retry exceeded, 0x22 remote aborted.

## Appendix D. Event types

| Type | Name | | Type | Name |
|---|---|---|---|---|
| 0x00 | completion | | 0x10 | WQ invalid request error |
| 0x01 | path migrated | | 0x11 | WQ access error |
| 0x02 | communication established | | 0x12 | SRQ catastrophic error |
| 0x03 | SQ drained | | 0x13 | SRQ QP last WQE |
| 0x04 | CQ error | | 0x14 | SRQ limit |
| 0x05 | WQ catastrophic error | | 0x18 | communication channel |
| 0x06 | EEC catastrophic error | | 0x19 | VEP update |
| 0x07 | path migration failed | | 0x1a | operation required |
| 0x08 | local catastrophic error | | 0x1b | fatal warning |
| 0x09 | port change (subtype 1 down, 4 active) | | 0x1c | FLR event |
| 0x0a | command completion | | 0x1d | port management change |
| 0x0e | ECC detect | | 0x3e | recoverable error |
| 0x0f | EQ overflow | | 0xff | none |

## Appendix E. Glossary

| Term | Meaning |
|---|---|
| A0 / B0 steering | Two firmware receive-steering schemes: A0 by port MAC table and a base QPN (SET_PORT RQP_CALC); B0 by MCG entries keyed on MAC (5.4) |
| AMGM | The overflow half of the MCG table, chained from hashed MGM entries |
| AUXC, ALTC, RDMARC | Per-QP auxiliary context tables in ICM (auxiliary state, alternate path, RDMA read responder cache); unused by Ethernet but must exist |
| BlueFlame | Write-combining send path that copies WQEs into BAR 2; not used |
| cMPT / dMPT | The two MPT tables: cMPT is indexed by object type and number (fixed layout), dMPT holds memory-region entries |
| CQ / CQE / CQN | Completion queue / completion queue entry / completion queue number |
| DCS | Device configuration space: BAR 0 |
| Doorbell record | A 32-bit counter in host memory the device reads by DMA (4.4) |
| EQ / EQE / EQN | Event queue / entry / number |
| FPP | "Force physical port" bit in QP params2 (used with port remapping) |
| FW area | Host memory lent to firmware with MAP_FA to run from |
| GID | 16-byte group identifier; for Ethernet steering it encodes port, steer type and MAC (5.4.1) |
| HCA | Host channel adapter: the device |
| HCR | Host Command Register: the 7-dword firmware command window (2.1) |
| ICM | Context memory: host memory lent to the device for its tables, addressed by ICM virtual addresses |
| L_Key | Local key naming a memory region in WQE data segments |
| MCG / MGM | Multicast group table / one of its entries |
| MPT | Memory protection table entry (memory region) |
| MTT | Memory translation table: page address list for queue buffers |
| op_mod | Opcode modifier field of a command |
| PD | Protection domain |
| PPF | Primary physical function |
| QP / QPN | Queue pair (send queue + receive queue) / its number |
| RQ / SQ | Receive queue / send queue of a QP |
| RST, INIT, RTR, RTS | QP states: reset, initialised, ready to receive, ready to send |
| SRQ | Shared receive queue (unused) |
| SYS_EQS | Firmware mode where EQ numbering comes from QUERY_FUNC (3.11) |
| TXBB | 64-byte send basic block; send WQEs are made of TXBBs |
| UAR | User access region: 4 KiB doorbell pages in BAR 2 |
| VPI | Virtual protocol interconnect: a port that can be InfiniBand or Ethernet |
| VSD | VLAN stripping disable (QP param3 bit 30) |
| WQE | Work queue entry (a send or receive descriptor) |

## Appendix F. Source index (Linux, function → sections)

| File | Functions | Sections |
|---|---|---|
| `reset.c` | `mlx4_reset` | 1.6 |
| `main.c` | `mlx4_pci_table`, `__mlx4_init_one`, `mlx4_load_one`, `mlx4_unload_one`, `mlx4_get_ownership`, `mlx4_free_ownership`, `mlx4_init_fw`, `mlx4_load_fw`, `mlx4_dev_cap`, `mlx4_init_hca`, `mlx4_init_icm`, `mlx4_init_cmpt_table`, `mlx4_free_icms`, `mlx4_close_hca`, `mlx4_close_fw`, `mlx4_setup_hca`, `choose_steering_mode`, `mlx4_query_func` | 1, 3, 4.1, 5.4, 6 |
| `cmd.c` | `mlx4_cmd_init`, `cmd_pending`, `mlx4_cmd_post`, `mlx4_cmd_poll`, `__mlx4_cmd`, `mlx4_status_to_errno`, `mlx4_closing_cmd_fatal_error`, `mlx4_alloc_cmd_mailbox` | 2 |
| `fw.c` | `mlx4_QUERY_FW`, `mlx4_map_cmd`, `mlx4_MAP_FA`, `mlx4_RUN_FW`, `mlx4_UNMAP_FA`, `mlx4_MOD_STAT_CFG`, `mlx4_QUERY_DEV_CAP`, `mlx4_QUERY_PORT`, `mlx4_QUERY_ADAPTER`, `mlx4_INIT_HCA`, `mlx4_SET_ICM_SIZE`, `mlx4_QUERY_FUNC`, `mlx4_INIT_PORT`, `mlx4_CLOSE_PORT`, `mlx4_CLOSE_HCA`, `mlx4_NOP` | 3, 5.5 |
| `icm.c` | `mlx4_alloc_icm`, `mlx4_MAP_ICM`, `mlx4_UNMAP_ICM`, `mlx4_MAP_ICM_AUX`, `mlx4_UNMAP_ICM_AUX`, `mlx4_table_get`, `mlx4_init_icm_table`, `mlx4_table_find` | 3.2, 3.8, 3.9, 4.2 |
| `profile.c` | `mlx4_make_profile` | 3.7 |
| `pd.c` | `mlx4_init_pd_table`, `mlx4_init_uar_table`, `mlx4_uar_alloc` | 1.4, 4.1 |
| `mr.c` | `mlx4_mtt_init`, `mlx4_mtt_addr`, `mlx4_write_mtt`, `mlx4_mr_alloc`, `mlx4_mr_enable`, `mlx4_mr_free`, `hw_index_to_key`, `mlx4_init_mr_table` | 4.1–4.3 |
| `alloc.c` | `mlx4_buf_direct_alloc`, `mlx4_db_alloc`, `mlx4_alloc_hwq_res` | 4.2, 4.4 |
| `eq.c` | `mlx4_create_eq`, `mlx4_free_eq`, `mlx4_MAP_EQ`, `eq_set_ci`, `next_eqe_sw`, `mlx4_eq_int`, `mlx4_get_eq_uar`, `mlx4_init_eq_table`, `mlx4_cleanup_eq_table` | 4.6 |
| `cq.c` | `mlx4_cq_alloc`, `mlx4_cq_free`, `mlx4_init_kernel_cqes` | 4.7 |
| `qp.c` | `__mlx4_qp_modify`, `mlx4_qp_to_ready`, `mlx4_qp_query`, `mlx4_init_qp_table`, `mlx4_CONF_SPECIAL_QP` | 3.12, 4.8 |
| `port.c` | `mlx4_SET_PORT_general`, `mlx4_SET_PORT_qpn_calc`, `mlx4_SET_PORT_user_mtu`, `mlx4_set_port_mac_table`, `__mlx4_register_mac`, `mlx4_SET_MCAST_FLTR` | 5.3, 5.4 |
| `mcg.c` | `mlx4_GID_HASH`, `mlx4_READ_ENTRY`, `mlx4_WRITE_ENTRY`, `mlx4_WRITE_PROMISC`, `find_entry`, `mlx4_qp_attach_common`, `add_promisc_qp`, `mlx4_unicast_attach`, `mlx4_multicast_attach` | 5.4 |
| `sense.c` | `mlx4_SENSE_PORT` | 5.1 |
| `catas.c` | `poll_catas`, `mlx4_start_catas_poll` | 2.8 |
| `en_main.c` | `mlx4_en_probe` | 4.3, 6 |
| `en_netdev.c` | `mlx4_en_start_port`, `mlx4_en_stop_port`, `mlx4_en_get_qp`, `mlx4_en_uc_steer_add`, `mlx4_en_set_promisc_mode`, `mlx4_en_do_multicast`, `mlx4_en_init_netdev` | 5, 6 |
| `en_resources.c` | `mlx4_en_fill_qp_context` | 4.8.4, 5.10 |
| `en_rx.c` | `mlx4_en_calc_rx_buf`, `mlx4_en_init_rx_desc`, `mlx4_en_activate_rx_rings`, `mlx4_en_config_rss_qp`, `mlx4_en_config_rss_steer`, `mlx4_en_process_rx_cq` | 4.8, 5.6 |
| `en_tx.c` | `mlx4_en_create_tx_ring`, `mlx4_en_activate_tx_ring`, `mlx4_en_xmit`, `mlx4_en_xmit_frame`, `build_inline_wqe`, `mlx4_en_process_tx_cq`, `mlx4_en_stamp_wqe`, `mlx4_en_xmit_doorbell` | 5.7 |
| `en_cq.c` | `mlx4_en_create_cq`, `mlx4_en_activate_cq`, `mlx4_en_arm_cq` | 4.7 |
| `en_port.c` | `mlx4_en_QUERY_PORT` | 3.5, 5.8 |
| `include/linux/mlx4/*.h` | `cmd.h` opcodes; `device.h` flags, events, opcodes, `mlx4_eqe`; `qp.h` QP context and WQE segments; `cq.h` CQE and CQ doorbells; `doorbell.h` UAR offsets | throughout |
