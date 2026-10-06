//! The last few suppressed trace lines (#16), so a failure can still show
//! the steps that led to it when the console is quiet. No UEFI here, so
//! `test/trace.rs` runs it on the host; `console.rs` owns the one instance.

use alloc::collections::VecDeque;
use alloc::string::String;

/// How many trace lines a failure replays: the commands and decisions just
/// before it (a whole bring-up is about a hundred lines).
pub const KEEP: usize = 16;

pub struct Ring {
    lines: VecDeque<String>,
    /// Lines pushed out since the last `clear` or `take`, so the replay can
    /// say that earlier steps are missing.
    dropped: usize,
}

impl Ring {
    pub const fn new() -> Self {
        Ring { lines: VecDeque::new(), dropped: 0 }
    }

    pub fn push(&mut self, line: String) {
        if self.lines.len() == KEEP {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(line);
    }

    /// Forget everything: the start of a new Start.
    pub fn clear(&mut self) {
        self.lines.clear();
        self.dropped = 0;
    }

    /// Take the lines, oldest first, and how many were dropped before them.
    /// The ring is empty afterwards, so a later failure replays only what
    /// came after this one.
    pub fn take(&mut self) -> (VecDeque<String>, usize) {
        (core::mem::take(&mut self.lines), core::mem::take(&mut self.dropped))
    }
}
