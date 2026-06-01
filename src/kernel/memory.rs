//! A deliberately small memory manager that powers Mirage's dynamic allocation
//! routines. The implementation is intentionally conservative but demonstrates
//! how `malloc`, `free`, and `mmap` style services could be layered on top of a
//! statically provisioned heap in a `no_std` kernel.

use core::{
    cmp,
    ptr::{self, NonNull},
};

use crate::kernel::process::ProcessId;
use crate::kernel::sync::SpinLock;

pub const PAGE_SIZE: usize = 4096;
pub const DEFAULT_HEAP_BYTES: usize = 128 * 1024;
pub const MAX_ALLOCATION_RECORDS: usize = 512;
pub const KERNEL_PROCESS_ID: ProcessId = ProcessId::new(0);

pub const PROT_READ: u32 = 0x1;
pub const PROT_WRITE: u32 = 0x2;
pub const PROT_EXECUTE: u32 = 0x4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocationKind {
    Heap,
    Mapping,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryProtection {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl MemoryProtection {
    pub const fn new(read: bool, write: bool, execute: bool) -> Self {
        Self {
            read,
            write,
            execute,
        }
    }

    pub const fn read_only() -> Self {
        Self::new(true, false, false)
    }

    pub const fn read_write() -> Self {
        Self::new(true, true, false)
    }

    pub const fn read_exec() -> Self {
        Self::new(true, false, true)
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self::new(
            (bits & PROT_READ) != 0,
            (bits & PROT_WRITE) != 0,
            (bits & PROT_EXECUTE) != 0,
        )
    }

    pub const fn bits(&self) -> u32 {
        (self.read as u32 * PROT_READ)
            | (self.write as u32 * PROT_WRITE)
            | (self.execute as u32 * PROT_EXECUTE)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AllocationRecord {
    owner: ProcessId,
    offset: usize,
    size: usize,
    kind: AllocationKind,
    protection: MemoryProtection,
}

impl AllocationRecord {
    const fn new(
        owner: ProcessId,
        offset: usize,
        size: usize,
        kind: AllocationKind,
        protection: MemoryProtection,
    ) -> Self {
        Self {
            owner,
            offset,
            size,
            kind,
            protection,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FreeRegion {
    offset: usize,
    size: usize,
}

impl FreeRegion {
    const fn new(offset: usize, size: usize) -> Self {
        Self { offset, size }
    }

    fn end(&self) -> usize {
        self.offset + self.size
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocationStats {
    pub allocated_bytes: usize,
    pub peak_allocated_bytes: usize,
}

pub struct MemoryManager<const HEAP_SIZE: usize, const MAX_AREAS: usize> {
    heap: [u8; HEAP_SIZE],
    bump_offset: usize,
    allocations: [Option<AllocationRecord>; MAX_AREAS],
    free_regions: [Option<FreeRegion>; MAX_AREAS],
    allocated_bytes: usize,
    peak_bytes: usize,
}

impl<const HEAP_SIZE: usize, const MAX_AREAS: usize> MemoryManager<HEAP_SIZE, MAX_AREAS> {
    pub const fn new() -> Self {
        Self {
            heap: [0; HEAP_SIZE],
            bump_offset: 0,
            allocations: [None; MAX_AREAS],
            free_regions: [None; MAX_AREAS],
            allocated_bytes: 0,
            peak_bytes: 0,
        }
    }

    pub fn malloc(&mut self, size: usize) -> Option<NonNull<u8>> {
        self.malloc_for(KERNEL_PROCESS_ID, size)
    }

    pub fn malloc_for(&mut self, owner: ProcessId, size: usize) -> Option<NonNull<u8>> {
        if size == 0 {
            return None;
        }

        let align = core::mem::size_of::<usize>();
        let actual_size = self.align_up(size, align)?;
        let offset = self.reserve(actual_size, align)?;
        let record = AllocationRecord::new(
            owner,
            offset,
            actual_size,
            AllocationKind::Heap,
            MemoryProtection::read_write(),
        );
        if self.record_allocation(record).is_none() {
            self.insert_free_region(FreeRegion::new(offset, actual_size));
            return None;
        }
        self.update_stats_on_alloc(actual_size);
        Some(unsafe { NonNull::new_unchecked(self.heap.as_mut_ptr().add(offset)) })
    }

    pub fn malloc_aligned(&mut self, size: usize, align: usize) -> Option<NonNull<u8>> {
        self.malloc_aligned_for(KERNEL_PROCESS_ID, size, align)
    }

    pub fn malloc_aligned_for(
        &mut self,
        owner: ProcessId,
        size: usize,
        align: usize,
    ) -> Option<NonNull<u8>> {
        if size == 0 || !Self::valid_alignment(align) {
            return None;
        }

        let actual_align = align.max(core::mem::size_of::<usize>());
        let actual_size = self.align_up(size, core::mem::size_of::<usize>())?;

        let offset = self.reserve(actual_size, actual_align)?;
        let record = AllocationRecord::new(
            owner,
            offset,
            actual_size,
            AllocationKind::Heap,
            MemoryProtection::read_write(),
        );
        if self.record_allocation(record).is_none() {
            self.insert_free_region(FreeRegion::new(offset, actual_size));
            return None;
        }
        self.update_stats_on_alloc(actual_size);
        Some(unsafe { NonNull::new_unchecked(self.heap.as_mut_ptr().add(offset)) })
    }

    pub fn realloc(&mut self, ptr: Option<NonNull<u8>>, new_size: usize) -> Option<NonNull<u8>> {
        self.realloc_for(KERNEL_PROCESS_ID, ptr, new_size)
    }

    pub fn realloc_for(
        &mut self,
        owner: ProcessId,
        ptr: Option<NonNull<u8>>,
        new_size: usize,
    ) -> Option<NonNull<u8>> {
        match (ptr, new_size) {
            (None, 0) => None,
            (None, size) => self.malloc_for(owner, size),
            (Some(p), 0) => {
                self.free_for(owner, p);
                None
            }
            (Some(p), size) => {
                let base = self.heap.as_ptr() as usize;
                let addr = p.as_ptr() as usize;
                if addr < base || addr >= base + HEAP_SIZE {
                    return None;
                }
                let offset = addr - base;
                let idx = self.find_allocation_index(owner, offset)?;
                let mut record = match self.allocations[idx] {
                    Some(r) => r,
                    None => return None,
                };
                if record.kind != AllocationKind::Heap {
                    return None;
                }

                let align = core::mem::size_of::<usize>();
                let aligned_new = self.align_up(size, align)?;

                if aligned_new <= record.size {
                    let leftover = record.size.saturating_sub(aligned_new);
                    if leftover > 0 {
                        let free_offset = record.offset + aligned_new;
                        self.insert_free_region(FreeRegion::new(free_offset, leftover));
                        self.update_stats_on_free(leftover);
                    }
                    record.size = aligned_new;
                    self.allocations[idx] = Some(record);
                    return Some(p);
                }

                let copy_len = cmp::min(record.size, size);
                let new_ptr = self.malloc_for(owner, size)?;
                unsafe {
                    ptr::copy_nonoverlapping(p.as_ptr(), new_ptr.as_ptr(), copy_len);
                }
                self.free_for(owner, p);
                Some(new_ptr)
            }
        }
    }

    pub fn free(&mut self, ptr: NonNull<u8>) -> bool {
        self.free_for(KERNEL_PROCESS_ID, ptr)
    }

    pub fn free_for(&mut self, owner: ProcessId, ptr: NonNull<u8>) -> bool {
        self.release(owner, ptr, Some(AllocationKind::Heap), None)
    }

    pub fn mmap(&mut self, length: usize, protection: MemoryProtection) -> Option<MappedRegion> {
        self.mmap_for(KERNEL_PROCESS_ID, length, protection)
    }

    pub fn mmap_for(
        &mut self,
        owner: ProcessId,
        length: usize,
        protection: MemoryProtection,
    ) -> Option<MappedRegion> {
        if length == 0 {
            return None;
        }

        let align = PAGE_SIZE;
        let actual_size = self.align_up(length, PAGE_SIZE)?;
        let offset = self.reserve(actual_size, align)?;
        let record = AllocationRecord::new(
            owner,
            offset,
            actual_size,
            AllocationKind::Mapping,
            protection,
        );
        if self.record_allocation(record).is_none() {
            self.insert_free_region(FreeRegion::new(offset, actual_size));
            return None;
        }
        self.update_stats_on_alloc(actual_size);
        let ptr = unsafe { NonNull::new_unchecked(self.heap.as_mut_ptr().add(offset)) };
        Some(MappedRegion {
            owner,
            ptr,
            length: actual_size,
            requested: length,
            protection,
            kind: AllocationKind::Mapping,
        })
    }

    pub fn munmap(&mut self, region: MappedRegion) -> bool {
        self.release(
            region.owner,
            region.ptr,
            Some(AllocationKind::Mapping),
            Some(region.length),
        )
    }

    pub fn munmap_ptr(&mut self, ptr: NonNull<u8>, length: usize) -> bool {
        self.munmap_ptr_for(KERNEL_PROCESS_ID, ptr, length)
    }

    pub fn munmap_ptr_for(&mut self, owner: ProcessId, ptr: NonNull<u8>, length: usize) -> bool {
        self.release(owner, ptr, Some(AllocationKind::Mapping), Some(length))
    }

    pub fn release_process(&mut self, owner: ProcessId) {
        let mut idx = 0;
        while idx < MAX_AREAS {
            if let Some(record) = self.allocations[idx] {
                if record.owner == owner {
                    self.allocations[idx] = None;
                    self.insert_free_region(FreeRegion::new(record.offset, record.size));
                    self.update_stats_on_free(record.size);
                }
            }
            idx += 1;
        }
    }

    pub fn statistics(&self) -> AllocationStats {
        AllocationStats {
            allocated_bytes: self.allocated_bytes,
            peak_allocated_bytes: self.peak_bytes,
        }
    }

    fn reserve(&mut self, size: usize, align: usize) -> Option<usize> {
        if let Some(offset) = self.reserve_from_free_list(size, align) {
            return Some(offset);
        }

        let aligned_offset = self.aligned_heap_offset(self.bump_offset, align)?;
        let end = aligned_offset.checked_add(size)?;
        if end > HEAP_SIZE {
            return None;
        }
        self.bump_offset = end;
        Some(aligned_offset)
    }

    fn reserve_from_free_list(&mut self, size: usize, align: usize) -> Option<usize> {
        let mut idx = 0;
        while idx < MAX_AREAS {
            if let Some(region) = self.free_regions[idx] {
                let aligned_start = self.aligned_heap_offset(region.offset, align)?;
                let end = aligned_start.checked_add(size)?;
                if end <= region.end() {
                    self.free_regions[idx] = None;
                    if aligned_start > region.offset {
                        let before = FreeRegion::new(region.offset, aligned_start - region.offset);
                        self.insert_free_region(before);
                    }
                    if end < region.end() {
                        let after = FreeRegion::new(end, region.end() - end);
                        self.insert_free_region(after);
                    }
                    return Some(aligned_start);
                }
            }
            idx += 1;
        }
        None
    }

    fn record_allocation(&mut self, record: AllocationRecord) -> Option<()> {
        let mut idx = 0;
        while idx < MAX_AREAS {
            if self.allocations[idx].is_none() {
                self.allocations[idx] = Some(record);
                return Some(());
            }
            idx += 1;
        }
        None
    }

    fn release(
        &mut self,
        owner: ProcessId,
        ptr: NonNull<u8>,
        expected_kind: Option<AllocationKind>,
        minimum_length: Option<usize>,
    ) -> bool {
        let base = self.heap.as_ptr() as usize;
        let addr = ptr.as_ptr() as usize;
        if addr < base || addr >= base + HEAP_SIZE {
            return false;
        }
        let offset = addr - base;
        if let Some(record) = self.remove_allocation(owner, offset, expected_kind, minimum_length) {
            self.insert_free_region(FreeRegion::new(record.offset, record.size));
            self.update_stats_on_free(record.size);
            true
        } else {
            false
        }
    }

    fn find_allocation_index(&self, owner: ProcessId, offset: usize) -> Option<usize> {
        let mut idx = 0;
        while idx < MAX_AREAS {
            if let Some(record) = self.allocations[idx] {
                if record.owner == owner && record.offset == offset {
                    return Some(idx);
                }
            }
            idx += 1;
        }
        None
    }

    fn remove_allocation(
        &mut self,
        owner: ProcessId,
        offset: usize,
        expected_kind: Option<AllocationKind>,
        minimum_length: Option<usize>,
    ) -> Option<AllocationRecord> {
        let mut idx = 0;
        while idx < MAX_AREAS {
            if let Some(record) = self.allocations[idx] {
                if record.owner == owner && record.offset == offset {
                    if let Some(kind) = expected_kind {
                        if record.kind != kind {
                            return None;
                        }
                    }
                    if let Some(length) = minimum_length {
                        if record.size < length {
                            return None;
                        }
                    }
                    self.allocations[idx] = None;
                    return Some(record);
                }
            }
            idx += 1;
        }
        None
    }

    fn insert_free_region(&mut self, region: FreeRegion) {
        if region.size == 0 {
            return;
        }

        let mut merged = region;
        let mut idx = 0;
        while idx < MAX_AREAS {
            if let Some(existing) = self.free_regions[idx] {
                if existing.end() == merged.offset {
                    merged = FreeRegion::new(existing.offset, existing.size + merged.size);
                    self.free_regions[idx] = None;
                } else if merged.end() == existing.offset {
                    merged = FreeRegion::new(merged.offset, merged.size + existing.size);
                    self.free_regions[idx] = None;
                }
            }
            idx += 1;
        }

        idx = 0;
        while idx < MAX_AREAS {
            if self.free_regions[idx].is_none() {
                self.free_regions[idx] = Some(merged);
                return;
            }
            idx += 1;
        }
        // If we run out of free slots we simply drop the region, effectively leaking it.
    }

    fn aligned_heap_offset(&self, minimum_offset: usize, align: usize) -> Option<usize> {
        if !Self::valid_alignment(align) {
            return None;
        }

        let base_remainder = (self.heap.as_ptr() as usize) % align;
        let offset_remainder = minimum_offset % align;
        let current_remainder = if offset_remainder == 0 {
            base_remainder
        } else if base_remainder >= align - offset_remainder {
            base_remainder - (align - offset_remainder)
        } else {
            base_remainder + offset_remainder
        };
        let padding = (align - current_remainder) % align;
        minimum_offset.checked_add(padding)
    }

    fn align_up(&self, value: usize, align: usize) -> Option<usize> {
        if !Self::valid_alignment(align) {
            return None;
        }

        let mask = align - 1;
        value.checked_add(mask).map(|aligned| aligned & !mask)
    }

    const fn valid_alignment(align: usize) -> bool {
        align != 0 && align.is_power_of_two()
    }

    fn update_stats_on_alloc(&mut self, size: usize) {
        self.allocated_bytes = self.allocated_bytes.saturating_add(size);
        if self.allocated_bytes > self.peak_bytes {
            self.peak_bytes = self.allocated_bytes;
        }
    }

    fn update_stats_on_free(&mut self, size: usize) {
        self.allocated_bytes = self.allocated_bytes.saturating_sub(size);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappedRegion {
    pub owner: ProcessId,
    pub ptr: NonNull<u8>,
    pub length: usize,
    pub requested: usize,
    pub protection: MemoryProtection,
    pub kind: AllocationKind,
}

impl MappedRegion {
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }
}

type KernelMemory = MemoryManager<DEFAULT_HEAP_BYTES, MAX_ALLOCATION_RECORDS>;

static MEMORY_MANAGER: SpinLock<KernelMemory> = SpinLock::new(MemoryManager::new());

pub fn malloc(size: usize) -> Option<NonNull<u8>> {
    malloc_for(KERNEL_PROCESS_ID, size)
}

pub fn malloc_for(owner: ProcessId, size: usize) -> Option<NonNull<u8>> {
    MEMORY_MANAGER.lock().malloc_for(owner, size)
}

pub fn malloc_aligned(size: usize, align: usize) -> Option<NonNull<u8>> {
    malloc_aligned_for(KERNEL_PROCESS_ID, size, align)
}

pub fn malloc_aligned_for(owner: ProcessId, size: usize, align: usize) -> Option<NonNull<u8>> {
    MEMORY_MANAGER.lock().malloc_aligned_for(owner, size, align)
}

pub fn realloc(ptr: Option<NonNull<u8>>, new_size: usize) -> Option<NonNull<u8>> {
    realloc_for(KERNEL_PROCESS_ID, ptr, new_size)
}

pub fn realloc_for(
    owner: ProcessId,
    ptr: Option<NonNull<u8>>,
    new_size: usize,
) -> Option<NonNull<u8>> {
    MEMORY_MANAGER.lock().realloc_for(owner, ptr, new_size)
}

pub fn free(ptr: NonNull<u8>) -> bool {
    free_for(KERNEL_PROCESS_ID, ptr)
}

pub fn free_for(owner: ProcessId, ptr: NonNull<u8>) -> bool {
    MEMORY_MANAGER.lock().free_for(owner, ptr)
}

pub fn mmap(length: usize, protection: MemoryProtection) -> Option<MappedRegion> {
    mmap_for(KERNEL_PROCESS_ID, length, protection)
}

pub fn mmap_for(
    owner: ProcessId,
    length: usize,
    protection: MemoryProtection,
) -> Option<MappedRegion> {
    MEMORY_MANAGER.lock().mmap_for(owner, length, protection)
}

pub fn munmap(region: MappedRegion) -> bool {
    munmap_ptr_for(region.owner, region.ptr, region.length)
}

pub fn munmap_ptr(ptr: NonNull<u8>, length: usize) -> bool {
    munmap_ptr_for(KERNEL_PROCESS_ID, ptr, length)
}

pub fn munmap_ptr_for(owner: ProcessId, ptr: NonNull<u8>, length: usize) -> bool {
    MEMORY_MANAGER.lock().munmap_ptr_for(owner, ptr, length)
}

pub fn release_process(owner: ProcessId) {
    MEMORY_MANAGER.lock().release_process(owner);
}

pub fn stats() -> AllocationStats {
    MEMORY_MANAGER.lock().statistics()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offset_of<const HEAP_SIZE: usize, const MAX_AREAS: usize>(
        manager: &MemoryManager<HEAP_SIZE, MAX_AREAS>,
        ptr: NonNull<u8>,
    ) -> usize {
        ptr.as_ptr() as usize - manager.heap.as_ptr() as usize
    }

    #[test]
    fn malloc_rolls_back_record_exhaustion_reservation() {
        let mut manager: MemoryManager<64, 2> = MemoryManager::new();
        let first = manager.malloc(8).expect("first allocation succeeds");
        let _second = manager.malloc(8).expect("second allocation succeeds");
        let stats_before = manager.statistics();

        assert!(manager.malloc(8).is_none());
        assert_eq!(manager.statistics(), stats_before);
        let rolled_back = manager.free_regions[0].expect("failed reservation was freed");

        assert!(manager.free(first));
        let reused = manager
            .malloc(8)
            .expect("rolled-back reservation can be reused");
        assert_eq!(offset_of(&manager, reused), rolled_back.offset);
    }

    #[test]
    fn malloc_aligned_rolls_back_record_exhaustion_reservation() {
        let mut manager: MemoryManager<128, 2> = MemoryManager::new();
        let first = manager
            .malloc_aligned(8, 16)
            .expect("first aligned allocation succeeds");
        let _second = manager
            .malloc_aligned(8, 16)
            .expect("second aligned allocation succeeds");
        let stats_before = manager.statistics();

        assert!(manager.malloc_aligned(8, 16).is_none());
        assert_eq!(manager.statistics(), stats_before);
        let rolled_back = manager.free_regions[0].expect("failed reservation was freed");

        assert!(manager.free(first));
        let reused = manager
            .malloc_aligned(8, 16)
            .expect("rolled-back aligned reservation can be reused");
        assert_eq!(offset_of(&manager, reused), rolled_back.offset);
    }

    #[test]
    fn mmap_rolls_back_record_exhaustion_reservation() {
        let mut manager: MemoryManager<{ PAGE_SIZE * 4 }, 2> = MemoryManager::new();
        let first = manager
            .mmap(1, MemoryProtection::read_only())
            .expect("first mapping succeeds");
        let _second = manager
            .mmap(1, MemoryProtection::read_write())
            .expect("second mapping succeeds");
        let stats_before = manager.statistics();

        assert!(manager.mmap(1, MemoryProtection::read_exec()).is_none());
        assert_eq!(manager.statistics(), stats_before);
        let rolled_back = manager.free_regions[0].expect("failed reservation was freed");

        assert!(manager.munmap(first));
        let reused = manager
            .mmap(1, MemoryProtection::read_exec())
            .expect("rolled-back mapping reservation can be reused");
        assert_eq!(offset_of(&manager, reused.ptr), rolled_back.offset);
    }

    #[test]
    fn malloc_and_free_cycle() {
        let mut manager: MemoryManager<4096, 16> = MemoryManager::new();
        let ptr = manager.malloc(32).expect("allocation succeeds");
        assert!(manager.free(ptr));
        assert_eq!(manager.statistics().allocated_bytes, 0);
    }

    #[test]
    fn mmap_produces_page_aligned_region() {
        let mut manager: MemoryManager<12288, 32> = MemoryManager::new();
        let heap = manager.malloc(1).expect("heap allocation succeeds");
        let region = manager
            .mmap(4096, MemoryProtection::read_only())
            .expect("mapping succeeds");
        assert_eq!(region.length, 4096);
        assert_eq!((region.ptr.as_ptr() as usize) % PAGE_SIZE, 0);
        assert!(manager.munmap(region));
        assert!(manager.free(heap));
    }

    #[test]
    fn freeing_unknown_pointer_fails() {
        let mut manager: MemoryManager<4096, 16> = MemoryManager::new();
        let bogus = unsafe { NonNull::new_unchecked(0x1000usize as *mut u8) };
        assert!(!manager.free(bogus));
    }

    #[test]
    fn malloc_aligned_respects_alignment() {
        let mut manager: MemoryManager<4096, 16> = MemoryManager::new();
        let ptr = manager
            .malloc_aligned(64, 64)
            .expect("aligned allocation succeeds");
        assert_eq!((ptr.as_ptr() as usize) % 64, 0);
        assert!(manager.free(ptr));
    }

    #[test]
    fn malloc_aligned_rejects_non_power_of_two_alignment() {
        let mut manager: MemoryManager<4096, 16> = MemoryManager::new();

        assert!(manager.malloc_aligned(64, 24).is_none());
        assert_eq!(manager.statistics().allocated_bytes, 0);
    }

    #[test]
    fn malloc_aligned_for_rejects_zero_alignment() {
        let mut manager: MemoryManager<4096, 16> = MemoryManager::new();

        assert!(manager
            .malloc_aligned_for(ProcessId::new(7), 64, 0)
            .is_none());
        assert_eq!(manager.statistics().allocated_bytes, 0);
    }

    #[test]
    fn align_up_returns_none_on_overflow() {
        let manager: MemoryManager<4096, 16> = MemoryManager::new();

        assert_eq!(manager.align_up(usize::MAX, 8), None);
    }

    #[test]
    fn malloc_aligned_from_free_list_respects_pointer_alignment() {
        let mut manager: MemoryManager<4096, 16> = MemoryManager::new();
        let ptr = manager.malloc(256).expect("allocation succeeds");
        assert!(manager.free(ptr));

        let aligned = manager
            .malloc_aligned(32, 128)
            .expect("aligned allocation succeeds");
        assert_eq!((aligned.as_ptr() as usize) % 128, 0);
        assert!(manager.free(aligned));
    }

    #[test]
    fn realloc_expands_and_preserves_contents() {
        let mut manager: MemoryManager<4096, 16> = MemoryManager::new();
        let ptr = manager.malloc(16).expect("allocation succeeds");
        unsafe {
            for i in 0..16 {
                ptr.as_ptr().add(i).write(i as u8);
            }
        }
        let new_ptr = manager
            .realloc(Some(ptr), 64)
            .expect("reallocation succeeds");
        unsafe {
            for i in 0..16 {
                assert_eq!(new_ptr.as_ptr().add(i).read(), i as u8);
            }
        }
        assert!(manager.free(new_ptr));
    }

    #[test]
    fn realloc_shrinks_in_place() {
        let mut manager: MemoryManager<4096, 16> = MemoryManager::new();
        let ptr = manager.malloc(64).expect("allocation succeeds");
        let stats_before = manager.statistics();
        let new_ptr = manager
            .realloc(Some(ptr), 16)
            .expect("reallocation succeeds");
        assert_eq!(new_ptr, ptr);
        let stats_after = manager.statistics();
        assert!(stats_after.allocated_bytes <= stats_before.allocated_bytes);
        assert!(manager.free(new_ptr));
    }

    #[test]
    fn realloc_zero_size_frees_allocation() {
        let mut manager: MemoryManager<4096, 16> = MemoryManager::new();
        let ptr = manager.malloc(32).expect("allocation succeeds");
        assert!(manager.realloc(Some(ptr), 0).is_none());
        assert_eq!(manager.statistics().allocated_bytes, 0);
    }

    #[test]
    fn allocations_are_owned_by_process() {
        let mut manager: MemoryManager<4096, 16> = MemoryManager::new();
        let owner = ProcessId::new(7);
        let other = ProcessId::new(8);
        let ptr = manager
            .malloc_for(owner, 32)
            .expect("process allocation succeeds");

        assert!(!manager.free_for(other, ptr));
        assert!(manager.free_for(owner, ptr));
    }

    #[test]
    fn release_process_reclaims_owned_records() {
        let mut manager: MemoryManager<8192, 16> = MemoryManager::new();
        let owner = ProcessId::new(7);
        let _heap = manager
            .malloc_for(owner, 32)
            .expect("process allocation succeeds");
        let _mapping = manager
            .mmap_for(owner, 4096, MemoryProtection::read_only())
            .expect("process mapping succeeds");

        manager.release_process(owner);

        assert_eq!(manager.statistics().allocated_bytes, 0);
    }
}
