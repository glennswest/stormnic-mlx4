//! Memory the device reads and writes: zeroed, page-aligned boot-services
//! pages, mapped as one PCI bus-master common buffer. Everything the device
//! sees is big-endian (spec 0.3), so the accessors swap.

use core::ffi::c_void;
use core::ptr::NonNull;

use crate::pci::PciIo;

pub const PAGE: usize = 4096;

/// A view of DMA memory: host pointer, length and device address. It is
/// `Copy` so queues can keep one while the owning `DmaBuf` sits in the list
/// of memory lent to the device (`fw::Hca::lent`); that list outlives every
/// view, since it is freed only once the device can no longer touch it.
#[derive(Clone, Copy)]
pub struct Mem {
    host: NonNull<u8>,
    len: usize,
    /// Device (bus) address of byte 0.
    pub dev: u64,
}

pub struct DmaBuf {
    mem: Mem,
    pub pages: usize,
    mapping: *mut c_void,
}

impl core::ops::Deref for DmaBuf {
    type Target = Mem;
    fn deref(&self) -> &Mem {
        &self.mem
    }
}

impl DmaBuf {
    /// `bytes` rounded up to whole pages, zeroed and mapped.
    pub fn new(pci: &mut PciIo, bytes: usize) -> uefi::Result<Self> {
        let pages = bytes.div_ceil(PAGE).max(1);
        let host = pci.allocate_pages(pages)?;
        // SAFETY: freshly allocated, `pages` pages long.
        unsafe { host.as_ptr().write_bytes(0, pages * PAGE) };
        match pci.map_common(host, pages * PAGE) {
            Ok((dev, mapping)) => Ok(DmaBuf { mem: Mem { host, len: pages * PAGE, dev }, pages, mapping }),
            Err(e) => {
                // SAFETY: never mapped, so the device never saw it.
                let _ = unsafe { pci.free_pages(pages, host) };
                Err(e)
            }
        }
    }

    pub fn mem(&self) -> Mem {
        self.mem
    }

    /// Unmap and free.
    ///
    /// # Safety
    /// The device must no longer use the buffer: its command has completed,
    /// or the firmware that owned it has been unmapped (UNMAP_*) or reset.
    pub unsafe fn free(self, pci: &mut PciIo) {
        let _ = pci.unmap(self.mapping);
        let _ = unsafe { pci.free_pages(self.pages, self.mem.host) };
    }
}

impl Mem {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn zero(&self) {
        self.fill(0);
    }

    pub fn fill(&self, b: u8) {
        // SAFETY: within the allocation.
        unsafe { self.host.as_ptr().write_bytes(b, self.len) };
    }

    fn at<T>(&self, off: usize) -> *mut T {
        assert!(off + size_of::<T>() <= self.len);
        // SAFETY: bounds checked above; the pointer is only used unaligned-safe below.
        unsafe { self.host.as_ptr().add(off).cast() }
    }

    // Volatile, because the device writes this memory behind the compiler's
    // back. Writes take `&self`: this is shared memory, not a Rust object.
    pub fn u8(&self, off: usize) -> u8 {
        unsafe { self.at::<u8>(off).read_volatile() }
    }
    pub fn be16(&self, off: usize) -> u16 {
        u16::from_be_bytes([self.u8(off), self.u8(off + 1)])
    }
    pub fn be32(&self, off: usize) -> u32 {
        (u32::from(self.be16(off)) << 16) | u32::from(self.be16(off + 2))
    }
    pub fn be64(&self, off: usize) -> u64 {
        (u64::from(self.be32(off)) << 32) | u64::from(self.be32(off + 4))
    }
    pub fn set_u8(&self, off: usize, v: u8) {
        unsafe { self.at::<u8>(off).write_volatile(v) }
    }
    pub fn set_be16(&self, off: usize, v: u16) {
        self.write(off, &v.to_be_bytes());
    }
    pub fn set_be32(&self, off: usize, v: u32) {
        self.write(off, &v.to_be_bytes());
    }
    pub fn set_be64(&self, off: usize, v: u64) {
        self.set_be32(off, (v >> 32) as u32);
        self.set_be32(off + 4, v as u32);
    }
    pub fn write(&self, off: usize, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            self.set_u8(off + i, b);
        }
    }
    pub fn read(&self, off: usize, out: &mut [u8]) {
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.u8(off + i);
        }
    }
    /// A sub-range, `len` bytes from `off`.
    pub fn slice(&self, off: usize, len: usize) -> Mem {
        assert!(off + len <= self.len);
        // SAFETY: within the allocation (checked above).
        Mem { host: unsafe { self.host.add(off) }, len, dev: self.dev + off as u64 }
    }
}

/// One entry of the page-list mailbox (spec 3.2): a block of `2^log` bytes,
/// at least 4 KiB, whose device address (and ICM virtual address, for
/// MAP_ICM) is aligned to its own size.
#[derive(Clone, Copy)]
pub struct Block {
    pub virt: u64,
    pub phys: u64,
    pub log: u32,
}

/// Split a device-contiguous range into the largest naturally aligned
/// power-of-two blocks (spec 3.2 allows this in place of Linux's uniform
/// split). `virt` is the ICM virtual address for MAP_ICM, whose alignment
/// then limits the blocks too; `None` for MAP_FA and MAP_ICM_AUX, whose
/// entries carry virtual address 0. All values are 4 KiB multiples.
pub fn blocks(virt: Option<u64>, phys: u64, size: u64) -> impl Iterator<Item = Block> {
    let mut off = 0u64;
    core::iter::from_fn(move || {
        if off >= size {
            return None;
        }
        let p = phys + off;
        let v = virt.map(|v| v + off);
        // `1 << 63` keeps trailing_zeros finite when every address is 0.
        let align = (p | v.unwrap_or(0) | (1 << 63)).trailing_zeros();
        let left = size - off;
        let log = align.min(63 - left.leading_zeros());
        off += 1 << log;
        Some(Block { virt: v.unwrap_or(0), phys: p, log })
    })
}
