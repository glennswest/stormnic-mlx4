//! stormnic-mlx4: an `EFI_SIMPLE_NETWORK_PROTOCOL` driver for ConnectX-3, in Rust.
//!
//! The image is an EFI boot-service driver (see `build.rs`). Its entry point
//! installs `EFI_DRIVER_BINDING_PROTOCOL` on its own image handle and returns;
//! the firmware's `ConnectController` (stormbootx runs one after loading the
//! media's drivers) then offers it every controller.
//!
//! `Supported` takes a PCI function whose vendor/device is a ConnectX-3
//! (15b3:1003) or ConnectX-3 Pro (15b3:1007) and that no other driver holds.
//! A platform's own driver always wins: a NIC that already carries an SNP, or
//! whose PCI I/O another driver has opened `BY_DRIVER`, is left alone.
//!
//! Scaffold: `Start` logs the bind, then releases the NIC and returns
//! `UNSUPPORTED`, because there is no bring-up yet (#2, #3) and holding the
//! device without producing an SNP would only keep another driver off it.
#![no_main]
#![no_std]

extern crate alloc;

mod pci;

use uefi::boot::{self, OpenProtocolAttributes, OpenProtocolParams, ScopedProtocol};
use uefi::driver::{self, Driver};
use uefi::prelude::*;
use uefi::proto::device_path::DevicePath;
use uefi::proto::network::snp::SimpleNetwork;
use uefi::Handle;

use pci::PciIo;

const VENDOR_MELLANOX: u16 = 0x15b3;

/// The devices this driver takes, and what to call them in the log.
const DEVICES: &[(u16, &str)] = &[(0x1003, "ConnectX-3"), (0x1007, "ConnectX-3 Pro")];

fn model(vendor: u16, device: u16) -> Option<&'static str> {
    if vendor != VENDOR_MELLANOX {
        return None;
    }
    DEVICES.iter().find(|(d, _)| *d == device).map(|(_, n)| *n)
}

/// Open the controller's PCI I/O. `GetProtocol` only looks; `ByDriver` claims
/// it, and fails with `ACCESS_DENIED` when another driver already has.
fn open_pci(
    agent: Handle,
    controller: Handle,
    attributes: OpenProtocolAttributes,
) -> uefi::Result<ScopedProtocol<PciIo>> {
    // SAFETY: the interface is only used for the duration of the callback,
    // and the firmware serialises driver-binding calls.
    unsafe {
        boot::open_protocol::<PciIo>(
            OpenProtocolParams { handle: controller, agent, controller: Some(controller) },
            attributes,
        )
    }
}

/// A ConnectX-3 on this controller: where it is and what it is. `None` for
/// anything else, which is every other handle in the system, so no logging.
fn identify(agent: Handle, controller: Handle) -> Option<(pci::Location, u16, &'static str)> {
    let mut pci = open_pci(agent, controller, OpenProtocolAttributes::GetProtocol).ok()?;
    let (vendor, device) = pci.ids().ok()?;
    let name = model(vendor, device)?;
    let loc = pci.location().ok()?;
    Some((loc, device, name))
}

struct Mlx4;

impl Driver for Mlx4 {
    fn supported(
        &mut self,
        agent: Handle,
        controller: Handle,
        _remaining: Option<&DevicePath>,
    ) -> uefi::Result {
        let Some((loc, device, name)) = identify(agent, controller) else {
            return Err(Status::UNSUPPORTED.into());
        };
        let tag = || uefi::println!("stormnic-mlx4: {loc} {VENDOR_MELLANOX:04x}:{device:04x} {name}:");

        let snp = OpenProtocolParams { handle: controller, agent, controller: None };
        if boot::test_protocol::<SimpleNetwork>(snp).unwrap_or(false) {
            tag();
            uefi::println!("  Supported: already has an SNP, leaving it to the platform's driver");
            return Err(Status::UNSUPPORTED.into());
        }

        // Claim and immediately release: Supported must leave no trace.
        match open_pci(agent, controller, OpenProtocolAttributes::ByDriver) {
            Ok(_pci) => {
                tag();
                uefi::println!("  Supported: yes");
                Ok(())
            }
            Err(e) => {
                tag();
                uefi::println!("  Supported: no, PCI I/O is held ({:?})", e.status());
                Err(e.status().into())
            }
        }
    }

    fn start(
        &mut self,
        agent: Handle,
        controller: Handle,
        _remaining: Option<&DevicePath>,
    ) -> uefi::Result {
        let mut pci = open_pci(agent, controller, OpenProtocolAttributes::ByDriver).map_err(|e| {
            uefi::println!("stormnic-mlx4: Start: cannot claim PCI I/O ({:?})", e.status());
            e
        })?;
        let (vendor, device) = pci.ids()?;
        let loc = pci.location()?;
        let name = model(vendor, device).unwrap_or("?");
        uefi::println!("stormnic-mlx4: {loc} {vendor:04x}:{device:04x} {name}:");
        uefi::println!("  Start: bound; no firmware bring-up yet, releasing the NIC");
        // `pci` drops here, closing the BY_DRIVER open.
        Err(Status::UNSUPPORTED.into())
    }

    fn stop(&mut self, _agent: Handle, _controller: Handle) -> uefi::Result {
        // Start never succeeds yet, so there is nothing held to release.
        uefi::println!("stormnic-mlx4: Stop");
        Ok(())
    }
}

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();
    let version = env!("CARGO_PKG_VERSION");
    match driver::install(Mlx4, None) {
        Ok(()) => {
            uefi::println!(
                "stormnic-mlx4 {version}: driver binding installed (15b3:1003 ConnectX-3, 15b3:1007 ConnectX-3 Pro)"
            );
            Status::SUCCESS
        }
        Err(e) => {
            uefi::println!("stormnic-mlx4 {version}: driver binding not installed: {:?}", e.status());
            e.status()
        }
    }
}
