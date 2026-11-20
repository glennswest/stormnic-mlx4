//! The order a port child is taken down in, kept apart from the boot
//! services so the host tests (`test/teardown.rs`) can drive it against
//! simulated firmware refusals (review finding 04121d04251f, #34).
//!
//! The child's memory holds the SNP and the device path the firmware points
//! at, so the caller may free it only when both are uninstalled. Any
//! refusal puts back what was already taken down and says "keep it".

/// The boot-service steps for one child. A `Err` is the firmware's refusal.
pub trait Steps {
    type Error: Copy;
    /// CloseProtocol of the parent's PCI I/O, opened by the child.
    fn close_pci(&mut self);
    /// OpenProtocol of the parent's PCI I/O by the child again.
    fn reopen_pci(&mut self);
    fn uninstall_snp(&mut self) -> Result<(), Self::Error>;
    fn reinstall_snp(&mut self) -> Result<(), Self::Error>;
    fn uninstall_path(&mut self) -> Result<(), Self::Error>;
    /// CloseEvent of `WaitForPacket`; only once the SNP is gone for good.
    fn close_event(&mut self);
}

/// What became of the child.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome<E> {
    /// Both protocols are gone and the event closed: the child may be freed.
    Gone,
    /// The SNP uninstall was refused (e.g. MNP holds it); the child is as it was.
    SnpRefused(E),
    /// The device path uninstall was refused. The SNP was installed again
    /// (`reinstalled` says whether that worked); the child must not be freed.
    PathRefused { err: E, reinstalled: Result<(), E> },
}

impl<E> Outcome<E> {
    /// May the caller free the child?
    pub fn freeable(&self) -> bool {
        matches!(self, Outcome::Gone)
    }
}

pub fn take_down<S: Steps>(s: &mut S) -> Outcome<S::Error> {
    s.close_pci();
    if let Err(e) = s.uninstall_snp() {
        s.reopen_pci();
        return Outcome::SnpRefused(e);
    }
    if let Err(e) = s.uninstall_path() {
        // The firmware still points at the path in the child's memory: put
        // the SNP back so the handle is whole again, and keep the child.
        let reinstalled = s.reinstall_snp();
        s.reopen_pci();
        return Outcome::PathRefused { err: e, reinstalled };
    }
    s.close_event();
    Outcome::Gone
}
