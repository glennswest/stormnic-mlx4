//! Link the image as an EFI boot-service driver rather than an application.
//!
//! The subsystem is what makes the firmware keep the image's code and data
//! resident after the entry point returns; the driver binding it installs
//! points into them. `uefi::driver::install` checks it and refuses to install
//! from an application image.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("uefi") {
        println!("cargo::rustc-link-arg=/SUBSYSTEM:EFI_BOOT_SERVICE_DRIVER");
        // A PE timestamp derived from the image's contents, not the build
        // time, so the same source gives the same bytes (#13).
        println!("cargo::rustc-link-arg=/Brepro");
        // No PDB and no CodeView debug directory: rustc passes /DEBUG even
        // with `strip = true`, and lld then records a PDB GUID hashed from the
        // PDB, which holds the build's absolute paths, so the image (and its
        // /Brepro timestamp) followed the build directory (#13). Firmware never
        // reads a PDB. This comes after rustc's /DEBUG, and the last one wins.
        println!("cargo::rustc-link-arg=/DEBUG:NONE");
    }
}
