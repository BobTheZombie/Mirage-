//! Central kernel MMIO mapping helpers.
//!
//! Drivers must map PCI BARs through this module and use the returned virtual
//! address. The HHDM is valid for RAM translations, but MMIO BARs are not
//! assumed to be present in the bootloader direct map.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::x86_64::paging::{self, PageFlags, PagingError};
use crate::kernel::memory::PAGE_SIZE;

const MMIO_VIRTUAL_BASE: u64 = 0xffff_a000_0000_0000;
const MMIO_VIRTUAL_LIMIT: u64 = 0xffff_a100_0000_0000;

static NEXT_MMIO_VIRT: AtomicU64 = AtomicU64::new(MMIO_VIRTUAL_BASE);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysAddr(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtAddr(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioRegion {
    pub phys: PhysAddr,
    pub virt: VirtAddr,
    pub len: usize,
}

impl MmioRegion {
    pub const fn end_virt(self) -> Option<u64> {
        self.virt.0.checked_add(self.len as u64)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioFlags(u8);

impl MmioFlags {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const DEVICE: Self = Self(Self::READ.0 | Self::WRITE.0 | (1 << 2));

    pub const fn writable(self) -> bool {
        (self.0 & Self::WRITE.0) != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MmioError {
    ZeroLength,
    AddressOverflow,
    VirtualSpaceExhausted,
    MapFailed(PagingError),
    VerifyFailed,
    VerifyNotWritable,
    VerifyUserAccessible,
}

pub const fn page_align_down(address: u64) -> u64 {
    address & !((PAGE_SIZE as u64) - 1)
}

pub const fn page_offset(address: u64) -> u64 {
    address & ((PAGE_SIZE as u64) - 1)
}

pub fn page_span_len(phys: PhysAddr, len: usize) -> Result<(u64, u64, u64), MmioError> {
    if len == 0 {
        return Err(MmioError::ZeroLength);
    }
    let aligned = page_align_down(phys.0);
    let offset = page_offset(phys.0);
    let bytes = (len as u64)
        .checked_add(offset)
        .ok_or(MmioError::AddressOverflow)?;
    let pages = bytes
        .checked_add((PAGE_SIZE as u64) - 1)
        .ok_or(MmioError::AddressOverflow)?
        & !((PAGE_SIZE as u64) - 1);
    Ok((aligned, offset, pages))
}

pub fn map_mmio(phys: PhysAddr, len: usize, flags: MmioFlags) -> Result<MmioRegion, MmioError> {
    let (phys_aligned, offset, map_len) = page_span_len(phys, len)?;
    let virt_aligned = reserve_virtual_pages(map_len)?;
    let mut page_flags = PageFlags::NO_EXECUTE | PageFlags::CACHE_DISABLE;
    if flags.writable() {
        page_flags |= PageFlags::WRITABLE;
    }
    paging::map_range(virt_aligned, phys_aligned, map_len, page_flags)
        .map_err(MmioError::MapFailed)?;
    let region = MmioRegion {
        phys,
        virt: VirtAddr(
            virt_aligned
                .checked_add(offset)
                .ok_or(MmioError::AddressOverflow)?,
        ),
        len,
    };
    verify_mapped(region.virt, region.len, flags)?;
    Ok(region)
}

pub fn verify_mapped(virt: VirtAddr, len: usize, flags: MmioFlags) -> Result<(), MmioError> {
    if len == 0 {
        return Err(MmioError::ZeroLength);
    }
    let start = page_align_down(virt.0);
    let end = virt
        .0
        .checked_add(len as u64)
        .ok_or(MmioError::AddressOverflow)?;
    let end = end
        .checked_add((PAGE_SIZE as u64) - 1)
        .ok_or(MmioError::AddressOverflow)?
        & !((PAGE_SIZE as u64) - 1);
    let mut current = start;
    while current < end {
        let Some(walk) = paging::walk_kernel_page_tables(current) else {
            return Err(MmioError::VerifyFailed);
        };
        if walk.physical.is_none() || (walk.pte & 1) == 0 {
            return Err(MmioError::VerifyFailed);
        }
        if flags.writable() && (walk.pte & (1 << 1)) == 0 {
            return Err(MmioError::VerifyNotWritable);
        }
        if (walk.pte & (1 << 2)) != 0 {
            return Err(MmioError::VerifyUserAccessible);
        }
        current = current
            .checked_add(PAGE_SIZE as u64)
            .ok_or(MmioError::AddressOverflow)?;
    }
    Ok(())
}

fn reserve_virtual_pages(len: u64) -> Result<u64, MmioError> {
    let len = (len + (PAGE_SIZE as u64 - 1)) & !((PAGE_SIZE as u64) - 1);
    let start = NEXT_MMIO_VIRT.fetch_add(len, Ordering::SeqCst);
    let end = start.checked_add(len).ok_or(MmioError::AddressOverflow)?;
    if end > MMIO_VIRTUAL_LIMIT {
        return Err(MmioError::VirtualSpaceExhausted);
    }
    Ok(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmio_alignment_math_includes_offset() {
        let (aligned, offset, len) = page_span_len(PhysAddr(0xfebd_5123), 0x1000).unwrap();
        assert_eq!(aligned, 0xfebd_5000);
        assert_eq!(offset, 0x123);
        assert_eq!(len, 0x2000);
    }

    #[test]
    fn mmio_alignment_rejects_zero_len() {
        assert_eq!(
            page_span_len(PhysAddr(0x1000), 0),
            Err(MmioError::ZeroLength)
        );
    }
}
