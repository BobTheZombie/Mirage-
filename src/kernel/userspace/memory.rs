//! Minimal userspace virtual-memory facade for the first PID 1 loader.

use crate::kernel::{memory, process::ProcessId};
use mirage_mtss::AddressSpaceId;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct VirtAddr(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PhysAddr(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserStack {
    pub bottom: VirtAddr,
    pub top: VirtAddr,
    pub size: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserMapFlags {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
    pub user: bool,
}

impl UserMapFlags {
    pub const READ_EXEC: Self = Self::new(true, false, true);
    pub const READ_WRITE: Self = Self::new(true, true, false);
    pub const READ_ONLY: Self = Self::new(true, false, false);

    pub const fn new(read: bool, write: bool, execute: bool) -> Self {
        Self {
            read,
            write,
            execute,
            user: true,
        }
    }

    pub const fn protection(self) -> memory::MemoryProtection {
        memory::MemoryProtection::new(self.read, self.write, self.execute)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MmError {
    AddressSpaceUnavailable,
    InvalidAddress,
    InvalidLength,
    InvalidFlags,
    MappingUnavailable,
}

const USER_STACK_TOP: u64 = 0x0000_7fff_ff00_0000;
const USER_CANONICAL_LIMIT: u64 = 0x0000_8000_0000_0000;

pub fn create_user_address_space() -> Result<AddressSpaceId, MmError> {
    let root = memory::create_user_address_space(ProcessId::new(1))
        .ok_or(MmError::AddressSpaceUnavailable)?;
    Ok(AddressSpaceId::new(root))
}

pub fn map_user_region(
    address_space: AddressSpaceId,
    user_va: VirtAddr,
    _phys: PhysAddr,
    len: usize,
    flags: UserMapFlags,
) -> Result<(), MmError> {
    if address_space.raw() == 0 || !flags.user {
        return Err(MmError::InvalidFlags);
    }
    if len == 0 {
        return Err(MmError::InvalidLength);
    }
    if !is_page_aligned(user_va.0) || !is_canonical_user_range(user_va.0, len) {
        return Err(MmError::InvalidAddress);
    }
    memory::mmap_user_fixed(
        ProcessId::new(1),
        address_space.raw(),
        user_va.0,
        len,
        flags.protection(),
    )
    .map(|_| ())
    .ok_or(MmError::MappingUnavailable)
}

pub fn allocate_user_stack(
    address_space: AddressSpaceId,
    size: usize,
) -> Result<UserStack, MmError> {
    if address_space.raw() == 0 {
        return Err(MmError::AddressSpaceUnavailable);
    }
    if size == 0 {
        return Err(MmError::InvalidLength);
    }
    let aligned = align_up(size as u64, memory::PAGE_SIZE as u64) as usize;
    let bottom = USER_STACK_TOP
        .checked_sub(aligned as u64)
        .ok_or(MmError::InvalidAddress)?;
    memory::mmap_user_fixed(
        ProcessId::new(1),
        address_space.raw(),
        bottom,
        aligned,
        memory::MemoryProtection::read_write(),
    )
    .map(|_| UserStack {
        bottom: VirtAddr(bottom),
        top: VirtAddr(USER_STACK_TOP),
        size: aligned,
    })
    .ok_or(MmError::MappingUnavailable)
}

pub const fn is_canonical_user_range(start: u64, len: usize) -> bool {
    match start.checked_add(len as u64) {
        Some(end) => start < USER_CANONICAL_LIMIT && end <= USER_CANONICAL_LIMIT,
        None => false,
    }
}

pub const fn is_page_aligned(address: u64) -> bool {
    address & ((memory::PAGE_SIZE as u64) - 1) == 0
}

const fn align_up(value: u64, align: u64) -> u64 {
    value.saturating_add(align - 1) & !(align - 1)
}
