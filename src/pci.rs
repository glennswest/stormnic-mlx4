//! `EFI_PCI_IO_PROTOCOL` (UEFI spec, "PCI Bus Support"), the parts this driver
//! calls. uefi-raw has no binding for it; the layout here is the spec's, with
//! the members not used kept as opaque pointers so the offsets stay right.

use core::ffi::c_void;
use core::ptr::{self, NonNull};

use uefi::proto::unsafe_protocol;
use uefi::{Status, StatusExt};

/// `EFI_PCI_IO_PROTOCOL_WIDTH`.
const WIDTH_UINT16: u32 = 1;
const WIDTH_UINT32: u32 = 2;

/// `EFI_PCI_IO_PROTOCOL_ATTRIBUTE_OPERATION`.
const ATTR_GET: u32 = 0;
const ATTR_SET: u32 = 1;
const ATTR_ENABLE: u32 = 2;
const ATTR_SUPPORTED: u32 = 4;

/// `EFI_PCI_IO_ATTRIBUTE_*`, the ones used.
pub const ATTR_MEMORY: u64 = 0x0200;
pub const ATTR_BUS_MASTER: u64 = 0x0400;
pub const ATTR_DUAL_ADDRESS_CYCLE: u64 = 0x8000;

/// `EfiPciIoOperationBusMasterCommonBuffer`.
const MAP_COMMON_BUFFER: u32 = 2;
/// `AllocateAnyPages`, `EfiBootServicesData`.
const ALLOCATE_ANY_PAGES: u32 = 0;
const BOOT_SERVICES_DATA: u32 = 4;

type ConfigAccess = unsafe extern "efiapi" fn(
    this: *mut PciIo,
    width: u32,
    offset: u32,
    count: usize,
    buffer: *mut c_void,
) -> Status;

type MemAccess = unsafe extern "efiapi" fn(
    this: *mut PciIo,
    width: u32,
    bar: u8,
    offset: u64,
    count: usize,
    buffer: *mut c_void,
) -> Status;

#[repr(C)]
struct Pair<T> {
    read: T,
    write: T,
}

#[repr(C)]
#[unsafe_protocol("4cf5b200-68b8-4ca5-9eec-b23e3f50029a")]
pub struct PciIo {
    poll_mem: *const c_void,
    poll_io: *const c_void,
    mem: Pair<MemAccess>,
    io: [*const c_void; 2],
    pci: Pair<ConfigAccess>,
    copy_mem: *const c_void,
    map: unsafe extern "efiapi" fn(
        this: *mut PciIo,
        operation: u32,
        host: *mut c_void,
        bytes: *mut usize,
        device: *mut u64,
        mapping: *mut *mut c_void,
    ) -> Status,
    unmap: unsafe extern "efiapi" fn(this: *mut PciIo, mapping: *mut c_void) -> Status,
    allocate_buffer: unsafe extern "efiapi" fn(
        this: *mut PciIo,
        kind: u32,
        memory_type: u32,
        pages: usize,
        host: *mut *mut c_void,
        attributes: u64,
    ) -> Status,
    free_buffer:
        unsafe extern "efiapi" fn(this: *mut PciIo, pages: usize, host: *mut c_void) -> Status,
    flush: *const c_void,
    get_location: unsafe extern "efiapi" fn(
        this: *mut PciIo,
        segment: *mut usize,
        bus: *mut usize,
        device: *mut usize,
        function: *mut usize,
    ) -> Status,
    attributes: unsafe extern "efiapi" fn(
        this: *mut PciIo,
        operation: u32,
        attributes: u64,
        result: *mut u64,
    ) -> Status,
    get_bar_attributes: unsafe extern "efiapi" fn(
        this: *mut PciIo,
        bar: u8,
        supports: *mut u64,
        resources: *mut *mut c_void,
    ) -> Status,
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

// SAFETY (all methods): `self` is a live interface the firmware handed us, and
// every buffer passed holds exactly `count` elements of the requested width.
impl PciIo {
    /// One dword of configuration space.
    pub fn read_config32(&mut self, offset: u32) -> uefi::Result<u32> {
        let mut v = 0u32;
        unsafe { (self.pci.read)(self, WIDTH_UINT32, offset, 1, (&raw mut v).cast()) }
            .to_result_with_val(|| v)
    }

    pub fn write_config32(&mut self, offset: u32, mut v: u32) -> uefi::Result {
        unsafe { (self.pci.write)(self, WIDTH_UINT32, offset, 1, (&raw mut v).cast()) }.to_result()
    }

    pub fn read_config16(&mut self, offset: u32) -> uefi::Result<u16> {
        let mut v = 0u16;
        unsafe { (self.pci.read)(self, WIDTH_UINT16, offset, 1, (&raw mut v).cast()) }
            .to_result_with_val(|| v)
    }

    pub fn write_config16(&mut self, offset: u32, mut v: u16) -> uefi::Result {
        unsafe { (self.pci.write)(self, WIDTH_UINT16, offset, 1, (&raw mut v).cast()) }.to_result()
    }

    /// Vendor and device ID (config offset 0).
    pub fn ids(&mut self) -> uefi::Result<(u16, u16)> {
        let v = self.read_config32(0)?;
        Ok((v as u16, (v >> 16) as u16))
    }

    pub fn location(&mut self) -> uefi::Result<Location> {
        let (mut segment, mut bus, mut device, mut function) = (0, 0, 0, 0);
        unsafe { (self.get_location)(self, &mut segment, &mut bus, &mut device, &mut function) }
            .to_result_with_val(|| Location { segment, bus, device, function })
    }

    /// A 32-bit memory read from a BAR, as the CPU sees it (little-endian).
    pub fn mem_read32(&mut self, bar: u8, offset: u64) -> uefi::Result<u32> {
        let mut v = 0u32;
        unsafe { (self.mem.read)(self, WIDTH_UINT32, bar, offset, 1, (&raw mut v).cast()) }
            .to_result_with_val(|| v)
    }

    pub fn mem_write32(&mut self, bar: u8, offset: u64, mut v: u32) -> uefi::Result {
        unsafe { (self.mem.write)(self, WIDTH_UINT32, bar, offset, 1, (&raw mut v).cast()) }
            .to_result()
    }

    fn attributes(&mut self, op: u32, attributes: u64) -> uefi::Result<u64> {
        let mut r = 0u64;
        unsafe { (self.attributes)(self, op, attributes, &mut r) }.to_result_with_val(|| r)
    }

    /// The attributes currently set, to restore when the driver lets go.
    pub fn get_attributes(&mut self) -> uefi::Result<u64> {
        self.attributes(ATTR_GET, 0)
    }

    pub fn supported_attributes(&mut self) -> uefi::Result<u64> {
        self.attributes(ATTR_SUPPORTED, 0)
    }

    pub fn enable_attributes(&mut self, attributes: u64) -> uefi::Result {
        self.attributes(ATTR_ENABLE, attributes).map(|_| ())
    }

    pub fn set_attributes(&mut self, attributes: u64) -> uefi::Result {
        self.attributes(ATTR_SET, attributes).map(|_| ())
    }

    /// The length of a memory BAR, from the ACPI QWORD address space
    /// descriptor `GetBarAttributes` returns (ACPI spec, "QWord Address Space
    /// Descriptor": tag 0x8a, `_LEN` at byte 38; the list ends with tag 0x79).
    pub fn bar_size(&mut self, bar: u8) -> uefi::Result<u64> {
        let mut res: *mut c_void = ptr::null_mut();
        unsafe { (self.get_bar_attributes)(self, bar, ptr::null_mut(), &mut res) }.to_result()?;
        let Some(res) = NonNull::new(res.cast::<u8>()) else {
            return Err(Status::NOT_FOUND.into());
        };
        // SAFETY: the firmware returns a descriptor list it allocated from
        // pool; it is read up to its end tag and then freed.
        let size = unsafe {
            let p = res.as_ptr();
            if *p == 0x8a {
                Some(ptr::read_unaligned(p.add(38).cast::<u64>()))
            } else {
                None
            }
        };
        let _ = unsafe { uefi::boot::free_pool(res) };
        size.ok_or_else(|| Status::UNSUPPORTED.into())
    }

    /// `pages` 4 KiB pages of boot-services memory the device may use,
    /// page-aligned, below 4 GiB (no `DUAL_ADDRESS_CYCLE` attribute).
    pub fn allocate_pages(&mut self, pages: usize) -> uefi::Result<NonNull<u8>> {
        let mut host: *mut c_void = ptr::null_mut();
        unsafe {
            (self.allocate_buffer)(self, ALLOCATE_ANY_PAGES, BOOT_SERVICES_DATA, pages, &mut host, 0)
        }
        .to_result()?;
        NonNull::new(host.cast()).ok_or_else(|| Status::OUT_OF_RESOURCES.into())
    }

    /// # Safety
    /// `host` came from `allocate_pages(pages)` and the device no longer uses it.
    pub unsafe fn free_pages(&mut self, pages: usize, host: NonNull<u8>) -> uefi::Result {
        unsafe { (self.free_buffer)(self, pages, host.as_ptr().cast()) }.to_result()
    }

    /// Map `bytes` at `host` as a bus-master common buffer: the device
    /// address, and the mapping token for `unmap` (which may be null). Fails
    /// unless the whole range maps as one contiguous device range.
    pub fn map_common(&mut self, host: NonNull<u8>, bytes: usize) -> uefi::Result<(u64, *mut c_void)> {
        let (mut n, mut dev, mut mapping) = (bytes, 0u64, ptr::null_mut());
        unsafe {
            (self.map)(self, MAP_COMMON_BUFFER, host.as_ptr().cast(), &mut n, &mut dev, &mut mapping)
        }
        .to_result()?;
        if n != bytes {
            let _ = self.unmap(mapping);
            return Err(Status::OUT_OF_RESOURCES.into());
        }
        Ok((dev, mapping))
    }

    pub fn unmap(&mut self, mapping: *mut c_void) -> uefi::Result {
        unsafe { (self.unmap)(self, mapping) }.to_result()
    }
}
