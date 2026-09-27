//! stormnic-mlx4: an `EFI_SIMPLE_NETWORK_PROTOCOL` driver for ConnectX-3, in Rust.
//!
//! Scaffold only. The driver binding, bring-up and SNP are the work plan in
//! CLAUDE.md. Until it lands, the entry point loads, logs, and returns
//! `UNSUPPORTED`, so a media that carries it boots exactly as without it.
#![no_main]
#![no_std]

use uefi::prelude::*;

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();
    uefi::println!("stormnic-mlx4 {}: scaffold, no driver installed", env!("CARGO_PKG_VERSION"));
    Status::UNSUPPORTED
}
