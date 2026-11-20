//! `src/teardown.rs` against simulated firmware refusals (review finding
//! 04121d04251f, #34): a child is freeable only when its SNP and its device
//! path are both uninstalled. Run by `scripts/test-host.sh`.
#[path = "../src/teardown.rs"]
mod teardown;
use teardown::{take_down, undo_install, Outcome, Steps};

/// A child handle in simulated firmware.
struct Fw {
    pci_open: bool,
    snp: bool,
    path: bool,
    event: bool,
    refuse_snp: bool,
    refuse_path: bool,
    refuse_reinstall: bool,
    log: Vec<&'static str>,
}

impl Fw {
    fn new() -> Fw {
        Fw {
            pci_open: true,
            snp: true,
            path: true,
            event: true,
            refuse_snp: false,
            refuse_path: false,
            refuse_reinstall: false,
            log: Vec::new(),
        }
    }
}

impl Steps for Fw {
    type Error = &'static str;
    fn close_pci(&mut self) {
        self.pci_open = false;
        self.log.push("close_pci");
    }
    fn reopen_pci(&mut self) {
        self.pci_open = true;
        self.log.push("reopen_pci");
    }
    fn uninstall_snp(&mut self) -> Result<(), &'static str> {
        self.log.push("uninstall_snp");
        if self.refuse_snp {
            return Err("ACCESS_DENIED");
        }
        self.snp = false;
        Ok(())
    }
    fn reinstall_snp(&mut self) -> Result<(), &'static str> {
        self.log.push("reinstall_snp");
        if self.refuse_reinstall {
            return Err("OUT_OF_RESOURCES");
        }
        self.snp = true;
        Ok(())
    }
    fn uninstall_path(&mut self) -> Result<(), &'static str> {
        self.log.push("uninstall_path");
        if self.refuse_path {
            return Err("ACCESS_DENIED");
        }
        self.path = false;
        Ok(())
    }
    fn close_event(&mut self) {
        assert!(!self.snp, "WaitForPacket closed while the SNP is installed");
        self.event = false;
        self.log.push("close_event");
    }
}

#[test]
fn everything_uninstalled_is_freeable() {
    let mut f = Fw::new();
    let o = take_down(&mut f);
    assert_eq!(o, Outcome::Gone);
    assert!(o.freeable());
    assert!(!f.snp && !f.path && !f.event && !f.pci_open);
}

#[test]
fn snp_refused_leaves_the_child_as_it_was() {
    let mut f = Fw::new();
    f.refuse_snp = true;
    let o = take_down(&mut f);
    assert_eq!(o, Outcome::SnpRefused("ACCESS_DENIED"));
    assert!(!o.freeable());
    assert!(f.snp && f.path && f.event && f.pci_open);
    assert!(!f.log.contains(&"uninstall_path"));
}

/// The finding: a refused device path uninstall must not let the child go.
#[test]
fn path_refused_keeps_the_child_and_puts_the_snp_back() {
    let mut f = Fw::new();
    f.refuse_path = true;
    let o = take_down(&mut f);
    assert_eq!(o, Outcome::PathRefused { err: "ACCESS_DENIED", reinstalled: Ok(()) });
    assert!(!o.freeable());
    assert!(f.path, "the path stays installed, so the child's memory must stay");
    assert!(f.snp && f.event && f.pci_open, "the handle is whole again");
    assert!(!f.log.contains(&"close_event"));
}

#[test]
fn path_refused_and_reinstall_refused_still_keeps_the_child() {
    let mut f = Fw::new();
    f.refuse_path = true;
    f.refuse_reinstall = true;
    let o = take_down(&mut f);
    assert_eq!(o, Outcome::PathRefused { err: "ACCESS_DENIED", reinstalled: Err("OUT_OF_RESOURCES") });
    assert!(!o.freeable());
    assert!(f.path && f.event);
}

/// Finding 0ccfe0c92f79 (#36): the PCI I/O open by the child failed, so the
/// install is undone, never touching a PCI open that does not exist.
fn not_opened() -> Fw {
    let mut f = Fw::new();
    f.pci_open = false;
    f
}

#[test]
fn undo_install_removes_both_protocols_and_the_event() {
    let mut f = not_opened();
    let o = undo_install(&mut f);
    assert_eq!(o, Outcome::Gone);
    assert!(o.freeable());
    assert!(!f.snp && !f.path && !f.event && !f.pci_open);
    assert_eq!(f.log, ["uninstall_snp", "uninstall_path", "close_event"]);
}

#[test]
fn undo_install_snp_refused_keeps_the_child_without_a_pci_open() {
    let mut f = not_opened();
    f.refuse_snp = true;
    let o = undo_install(&mut f);
    assert_eq!(o, Outcome::SnpRefused("ACCESS_DENIED"));
    assert!(!o.freeable());
    assert!(f.snp && f.path && f.event && !f.pci_open);
    assert!(!f.log.contains(&"reopen_pci") && !f.log.contains(&"close_pci"));
}

#[test]
fn undo_install_path_refused_keeps_the_child_and_puts_the_snp_back() {
    let mut f = not_opened();
    f.refuse_path = true;
    let o = undo_install(&mut f);
    assert_eq!(o, Outcome::PathRefused { err: "ACCESS_DENIED", reinstalled: Ok(()) });
    assert!(!o.freeable());
    assert!(f.snp && f.path && f.event && !f.pci_open);
    assert!(!f.log.contains(&"reopen_pci"));
}
