//! Console output (#16): one line per port by default, the bring-up trace
//! behind a verbose switch. The same switch as stormnic-ixgbe#22, so one
//! setting turns every stormnic driver verbose.
//!
//! - `say!`: always printed: the per-port summary, warnings and errors.
//! - `trace!`: printed only when verbose. Otherwise the line is kept in a
//!   ring of the last `trace::KEEP` lines (`trace.rs`).
//! - `alarm!`: a failure. When quiet, the kept lines are printed first, so
//!   the console still shows the steps that led to it (`fw::fail!` uses it).
//! - `note!(loud, …)`: `say!` when `loud`, else `trace!`.
//!
//! Verbose is on when the driver is built with `--features verbose`, or
//! when the EFI variable `StormnicVerbose` under `VENDOR` exists and its
//! first byte is not 0. The variable is read once, at the entry point.
//! stormbootx can set it (volatile, boot-service access) before it loads
//! drivers, and from the UEFI shell:
//! `setvar StormnicVerbose -guid ce1479a2-eab9-4176-b0ad-c909ea5b8e0b -bs =01`.

use alloc::format;
use alloc::string::String;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use uefi::boot::{self, Tpl};
use uefi::runtime::{self, VariableVendor};
use uefi::{cstr16, guid, Guid};

use crate::trace::Ring;

/// Vendor GUID of `StormnicVerbose`, shared by the stormnic drivers.
pub const VENDOR: Guid = guid!("ce1479a2-eab9-4176-b0ad-c909ea5b8e0b");

static VERBOSE: AtomicBool = AtomicBool::new(cfg!(feature = "verbose"));

struct Shared(UnsafeCell<Ring>);
// SAFETY: boot services run on one processor; `with_ring` keeps event
// notifications out while the ring is borrowed.
unsafe impl Sync for Shared {}
static RING: Shared = Shared(UnsafeCell::new(Ring::new()));

fn with_ring<T>(f: impl FnOnce(&mut Ring) -> T) -> T {
    // SAFETY: nothing logs above TPL_NOTIFY (the SNP runs at TPL_CALLBACK and
    // the ExitBootServices handler logs nothing), so this never lowers the
    // TPL; it keeps every other logger out until the guard drops.
    let _tpl = unsafe { boot::raise_tpl(Tpl::NOTIFY) };
    // SAFETY: the only borrow while the TPL is raised.
    f(unsafe { &mut *RING.0.get() })
}

/// Read the runtime switch. Called once, from the entry point.
pub fn init() {
    if verbose() {
        return;
    }
    let mut buf = [0u8; 16];
    if let Ok((data, _)) = runtime::get_variable(cstr16!("StormnicVerbose"), &VariableVendor(VENDOR), &mut buf) {
        if data.first().is_some_and(|b| *b != 0) {
            VERBOSE.store(true, Ordering::Relaxed);
        }
    }
}

pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// A new Start: a later failure replays only its own steps.
pub fn begin() {
    if !verbose() {
        with_ring(|r| r.clear());
    }
}

pub fn trace_line(args: core::fmt::Arguments) {
    if verbose() {
        uefi::println!("{args}");
    } else {
        let line = format!("{args}");
        with_ring(|r| r.push(line));
    }
}

/// Print the kept trace lines ahead of a failure; nothing when verbose,
/// where they were printed already.
pub fn replay() {
    if verbose() {
        return;
    }
    let (lines, dropped) = with_ring(|r| r.take());
    if lines.is_empty() {
        return;
    }
    uefi::println!(
        "stormnic-mlx4: the {} step(s) before the failure below{}:",
        lines.len(),
        if dropped > 0 { format!(" ({dropped} earlier not kept)") } else { String::new() }
    );
    for line in lines {
        uefi::println!("{line}");
    }
}

macro_rules! say {
    ($($t:tt)*) => { uefi::println!($($t)*) };
}
macro_rules! trace {
    ($($t:tt)*) => { $crate::console::trace_line(format_args!($($t)*)) };
}
macro_rules! alarm {
    ($($t:tt)*) => {{
        $crate::console::replay();
        uefi::println!($($t)*);
    }};
}
macro_rules! note {
    ($loud:expr, $($t:tt)*) => {
        if $loud { say!($($t)*) } else { trace!($($t)*) }
    };
}
