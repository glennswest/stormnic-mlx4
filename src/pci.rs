//! `EFI_PCI_IO_PROTOCOL` (UEFI spec, "PCI Bus Support"), the parts this driver
//! calls. uefi-raw has no binding for it; the layout here is the spec's, with
//! the members not used yet kept as opaque pointers so the offsets stay right.

use core::ffi::c_void;

use uefi::proto::unsafe_protocol;
use uefi::{Status, StatusExt};

/// `EFI_PCI_IO_PROTOCOL_WIDTH`, the one value used.
const WIDTH_UINT32: u32 = 2;

type Access = unsafe extern "efiapi" fn(
    this: *mut PciIo,
    width: u32,
    offset: u32,
    count: usize,
    buffer: *mut c_void,
) -> Status;

#[repr(C)]
struct AccessPair {
    read: Access,
    write: Access,
}

#[repr(C)]
#[unsafe_protocol("4cf5b200-68b8-4ca5-9eec-b23e3f50029a")]
pub struct PciIo {
    poll_mem: *const c_void,
    poll_io: *const c_void,
    mem: [*const c_void; 2],
    io: [*const c_void; 2],
    pci: AccessPair,
    copy_mem: *const c_void,
    map: *const c_void,
    unmap: *const c_void,
    allocate_buffer: *const c_void,
    free_buffer: *const c_void,
    flush: *const c_void,
    get_location: unsafe extern "efiapi" fn(
        this: *mut PciIo,
        segment: *mut usize,
        bus: *mut usize,
        device: *mut usize,
        function: *mut usize,
    ) -> Status,
    attributes: *const c_void,
    get_bar_attributes: *const c_void,
    set_bar_attributes: *const c_void,
    rom_size: u64,
    rom_image: *const c_void,
}

/// Where a function sits on the bus, printed the way `lspci` does.
#[derive(Clone, Copy)]
pub struct Location {
    pub segment: usize,
    pub bus: usize,
    pub device: usize,
    pub function: usize,
}

impl core::fmt::Display for Location {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:04x}:{:02x}:{:02x}.{:x}",
            self.segment, self.bus, self.device, self.function
        )
    }
}

impl PciIo {
    /// One dword of configuration space.
    pub fn read_config32(&mut self, offset: u32) -> uefi::Result<u32> {
        let mut v = 0u32;
        // SAFETY: `self` is a live interface the firmware handed us, and the
        // buffer holds exactly one element of the requested width.
        unsafe {
            (self.pci.read)(self, WIDTH_UINT32, offset, 1, (&raw mut v).cast())
        }
        .to_result_with_val(|| v)
    }

    /// Vendor and device ID (config offset 0).
    pub fn ids(&mut self) -> uefi::Result<(u16, u16)> {
        let v = self.read_config32(0)?;
        Ok((v as u16, (v >> 16) as u16))
    }

    pub fn location(&mut self) -> uefi::Result<Location> {
        let (mut segment, mut bus, mut device, mut function) = (0, 0, 0, 0);
        // SAFETY: as above; the out-pointers are valid locals.
        unsafe { (self.get_location)(self, &mut segment, &mut bus, &mut device, &mut function) }
            .to_result_with_val(|| Location { segment, bus, device, function })
    }
}
