//! Allocation and mapping service seam.

use core::ffi::c_void;
use core::ptr::NonNull;

use crate::kernel::memory::{
    self, AllocationStats, MappedRegion, MemoryProtection, PhysicalMemoryStats, PhysicalRegionKind,
};
use crate::kernel::process::ProcessId;

/// Kernel-internal adapter for allocation, ownership-tracked heap, and mappings.
pub trait MemoryService {
    fn allocate_physical_frame(&self) -> Option<u64>;

    fn deallocate_physical_frame(&self, frame: u64);

    fn reserve_physical_range(&self, start: u64, length: u64, kind: PhysicalRegionKind);

    fn physical_stats(&self) -> PhysicalMemoryStats;

    fn malloc(&self, size: usize) -> Option<NonNull<u8>>;

    fn malloc_for(&self, owner: ProcessId, size: usize) -> Option<NonNull<u8>>;

    fn malloc_aligned(&self, size: usize, align: usize) -> Option<NonNull<u8>>;

    fn malloc_aligned_for(
        &self,
        owner: ProcessId,
        size: usize,
        align: usize,
    ) -> Option<NonNull<u8>>;

    fn realloc(&self, ptr: Option<NonNull<u8>>, new_size: usize) -> Option<NonNull<u8>>;

    fn realloc_for(
        &self,
        owner: ProcessId,
        ptr: Option<NonNull<u8>>,
        new_size: usize,
    ) -> Option<NonNull<u8>>;

    fn free(&self, ptr: NonNull<u8>) -> bool;

    fn free_for(&self, owner: ProcessId, ptr: NonNull<u8>) -> bool;

    fn mmap(&self, length: usize, protection: MemoryProtection) -> Option<MappedRegion>;

    fn mmap_for(
        &self,
        owner: ProcessId,
        length: usize,
        protection: MemoryProtection,
    ) -> Option<MappedRegion>;

    fn munmap(&self, region: MappedRegion) -> bool;

    fn munmap_ptr(&self, ptr: NonNull<u8>, length: usize) -> bool;

    fn munmap_ptr_for(&self, owner: ProcessId, ptr: NonNull<u8>, length: usize) -> bool;

    fn release_process_memory(&self, owner: ProcessId);

    fn allocation_stats(&self) -> AllocationStats;

    fn raw_mmap_ptr(
        &self,
        owner: ProcessId,
        length: usize,
        protection: MemoryProtection,
    ) -> Option<*mut c_void> {
        self.mmap_for(owner, length, protection)
            .map(|region| region.as_ptr() as *mut c_void)
    }
}

/// Global memory adapter for the current kernel memory manager.
#[derive(Clone, Copy, Debug, Default)]
pub struct KernelMemoryService;

impl MemoryService for KernelMemoryService {
    fn allocate_physical_frame(&self) -> Option<u64> {
        memory::allocate_physical_frame()
    }

    fn deallocate_physical_frame(&self, frame: u64) {
        memory::deallocate_physical_frame(frame);
    }

    fn reserve_physical_range(&self, start: u64, length: u64, kind: PhysicalRegionKind) {
        memory::reserve_physical_range(start, length, kind);
    }

    fn physical_stats(&self) -> PhysicalMemoryStats {
        memory::physical_stats()
    }

    fn malloc(&self, size: usize) -> Option<NonNull<u8>> {
        memory::malloc(size)
    }

    fn malloc_for(&self, owner: ProcessId, size: usize) -> Option<NonNull<u8>> {
        memory::malloc_for(owner, size)
    }

    fn malloc_aligned(&self, size: usize, align: usize) -> Option<NonNull<u8>> {
        memory::malloc_aligned(size, align)
    }

    fn malloc_aligned_for(
        &self,
        owner: ProcessId,
        size: usize,
        align: usize,
    ) -> Option<NonNull<u8>> {
        memory::malloc_aligned_for(owner, size, align)
    }

    fn realloc(&self, ptr: Option<NonNull<u8>>, new_size: usize) -> Option<NonNull<u8>> {
        memory::realloc(ptr, new_size)
    }

    fn realloc_for(
        &self,
        owner: ProcessId,
        ptr: Option<NonNull<u8>>,
        new_size: usize,
    ) -> Option<NonNull<u8>> {
        memory::realloc_for(owner, ptr, new_size)
    }

    fn free(&self, ptr: NonNull<u8>) -> bool {
        memory::free(ptr)
    }

    fn free_for(&self, owner: ProcessId, ptr: NonNull<u8>) -> bool {
        memory::free_for(owner, ptr)
    }

    fn mmap(&self, length: usize, protection: MemoryProtection) -> Option<MappedRegion> {
        memory::mmap(length, protection)
    }

    fn mmap_for(
        &self,
        owner: ProcessId,
        length: usize,
        protection: MemoryProtection,
    ) -> Option<MappedRegion> {
        memory::mmap_for(owner, length, protection)
    }

    fn munmap(&self, region: MappedRegion) -> bool {
        memory::munmap(region)
    }

    fn munmap_ptr(&self, ptr: NonNull<u8>, length: usize) -> bool {
        memory::munmap_ptr(ptr, length)
    }

    fn munmap_ptr_for(&self, owner: ProcessId, ptr: NonNull<u8>, length: usize) -> bool {
        memory::munmap_ptr_for(owner, ptr, length)
    }

    fn release_process_memory(&self, owner: ProcessId) {
        memory::release_process(owner);
    }

    fn allocation_stats(&self) -> AllocationStats {
        memory::stats()
    }
}
