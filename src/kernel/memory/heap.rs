//! Frame-backed kernel heap reservation and commit tracking.
//!
//! The kernel heap lives at a fixed high-half virtual range, but pages are only
//! committed after the physical frame allocator and the frame-backed mapper are
//! both online.  This keeps post-boot heap growth on the normal physical-frame
//! ownership path instead of silently consuming the bootstrap static heap or the
//! early static page-table pool.

use crate::arch::x86_64::paging::{self, PageFlags};
use crate::kernel::memory::{
    allocate_physical_frame, deallocate_physical_frame, physical_allocator_initialized,
    MemoryProtection, EARLY_HEAP_BASE, EARLY_HEAP_BYTES, PAGE_SIZE,
};
use crate::kernel::sync::SpinLock;

/// Fixed high-half kernel heap base.
pub const KERNEL_HEAP_BASE: usize = EARLY_HEAP_BASE;
/// Total virtual address space reserved for the kernel heap.
pub const KERNEL_HEAP_RESERVED_BYTES: usize = EARLY_HEAP_BYTES;
/// Initial frame-backed heap commit performed during boot memory initialization.
pub const INITIAL_HEAP_COMMIT_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelHeapLayout {
    pub base: usize,
    pub reserved_bytes: usize,
    pub committed_bytes: usize,
    pub frame_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernelHeapError {
    PhysicalAllocatorNotInitialized,
    FrameBackedMapperNotInitialized,
    InvalidRange,
    AddressOverflow,
    OutOfFrames,
    MapFailed(paging::PagingError),
}

static HEAP_LAYOUT: SpinLock<Option<KernelHeapLayout>> = SpinLock::new(None);

/// Initialize the kernel heap after the frame allocator and frame-backed mapper
/// are online.
pub fn initialize() -> Result<KernelHeapLayout, KernelHeapError> {
    crate::kprintln!(
        "kernel heap: range {:#018x}..{:#018x}, reserved={} bytes",
        KERNEL_HEAP_BASE,
        KERNEL_HEAP_BASE.saturating_add(KERNEL_HEAP_RESERVED_BYTES),
        KERNEL_HEAP_RESERVED_BYTES
    );

    if !physical_allocator_initialized() {
        crate::kprintln!("kernel heap: initialization failed: physical allocator not initialized");
        return Err(KernelHeapError::PhysicalAllocatorNotInitialized);
    }
    if !paging::frame_backed_mapping_enabled() {
        crate::kprintln!("kernel heap: initialization failed: frame-backed mapper not initialized");
        return Err(KernelHeapError::FrameBackedMapperNotInitialized);
    }

    let committed = align_up(INITIAL_HEAP_COMMIT_BYTES).ok_or(KernelHeapError::AddressOverflow)?;
    if committed == 0 || committed > KERNEL_HEAP_RESERVED_BYTES {
        crate::kprintln!("kernel heap: initialization failed: invalid initial commit");
        return Err(KernelHeapError::InvalidRange);
    }

    let frames = commit_range(
        KERNEL_HEAP_BASE,
        0,
        committed,
        MemoryProtection::read_write(),
    )?;
    let layout = KernelHeapLayout {
        base: KERNEL_HEAP_BASE,
        reserved_bytes: KERNEL_HEAP_RESERVED_BYTES,
        committed_bytes: committed,
        frame_count: frames,
    };
    *HEAP_LAYOUT.lock() = Some(layout);

    crate::kprintln!(
        "kernel heap: initialized: base={:#018x}, committed={} bytes, reserved={} bytes, frames={}",
        layout.base,
        layout.committed_bytes,
        layout.reserved_bytes,
        layout.frame_count
    );
    Ok(layout)
}

/// Commit additional heap pages using real physical frames.
pub fn grow_committed(
    base: usize,
    reserved_bytes: usize,
    current_committed: usize,
    target_committed: usize,
    protection: MemoryProtection,
) -> Result<KernelHeapLayout, KernelHeapError> {
    if target_committed <= current_committed {
        return layout().ok_or(KernelHeapError::InvalidRange);
    }
    if base != KERNEL_HEAP_BASE || reserved_bytes != KERNEL_HEAP_RESERVED_BYTES {
        return Err(KernelHeapError::InvalidRange);
    }
    if target_committed > reserved_bytes {
        return Err(KernelHeapError::InvalidRange);
    }
    if !physical_allocator_initialized() {
        return Err(KernelHeapError::PhysicalAllocatorNotInitialized);
    }
    if !paging::frame_backed_mapping_enabled() {
        return Err(KernelHeapError::FrameBackedMapperNotInitialized);
    }

    let target = align_up(target_committed).ok_or(KernelHeapError::AddressOverflow)?;
    let current = align_up(current_committed).ok_or(KernelHeapError::AddressOverflow)?;
    let added_frames = commit_range(base, current, target, protection)?;

    let mut guard = HEAP_LAYOUT.lock();
    let mut layout = guard.unwrap_or(KernelHeapLayout {
        base,
        reserved_bytes,
        committed_bytes: current,
        frame_count: current / PAGE_SIZE,
    });
    layout.committed_bytes = target;
    layout.frame_count = layout.frame_count.saturating_add(added_frames);
    *guard = Some(layout);
    Ok(layout)
}

pub fn layout() -> Option<KernelHeapLayout> {
    *HEAP_LAYOUT.lock()
}

fn commit_range(
    base: usize,
    current_committed: usize,
    target_committed: usize,
    protection: MemoryProtection,
) -> Result<usize, KernelHeapError> {
    if target_committed < current_committed
        || current_committed % PAGE_SIZE != 0
        || target_committed % PAGE_SIZE != 0
    {
        return Err(KernelHeapError::InvalidRange);
    }

    let flags = page_flags_from_protection(protection);
    let mut next = current_committed;
    let mut frames = 0usize;
    while next < target_committed {
        let physical = allocate_physical_frame().ok_or_else(|| {
            rollback_committed_pages(base, current_committed, next);
            KernelHeapError::OutOfFrames
        })?;
        let Some(virtual_address) = (base as u64).checked_add(next as u64) else {
            deallocate_physical_frame(physical);
            rollback_committed_pages(base, current_committed, next);
            return Err(KernelHeapError::AddressOverflow);
        };
        if let Err(error) = paging::map_page(virtual_address, physical, flags) {
            deallocate_physical_frame(physical);
            rollback_committed_pages(base, current_committed, next);
            return Err(KernelHeapError::MapFailed(error));
        }
        frames = frames.saturating_add(1);
        next = next.saturating_add(PAGE_SIZE);
    }
    Ok(frames)
}

fn rollback_committed_pages(base: usize, start: usize, end: usize) {
    let mut offset = start;
    while offset < end {
        if let Ok(physical) = paging::unmap_page((base + offset) as u64) {
            deallocate_physical_frame(physical);
        }
        offset = offset.saturating_add(PAGE_SIZE);
    }
}

fn page_flags_from_protection(protection: MemoryProtection) -> PageFlags {
    let mut flags = PageFlags::GLOBAL;
    if protection.write {
        flags |= PageFlags::WRITABLE;
    }
    if !protection.execute {
        flags |= PageFlags::NO_EXECUTE;
    }
    flags
}

const fn align_up(value: usize) -> Option<usize> {
    match value.checked_add(PAGE_SIZE - 1) {
        Some(added) => Some(added & !(PAGE_SIZE - 1)),
        None => None,
    }
}
