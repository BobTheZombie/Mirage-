//! A deliberately small memory manager that powers Mirage's dynamic allocation
//! routines. The implementation is intentionally conservative but demonstrates
//! how `malloc`, `free`, and `mmap` style services could be layered on top of a
//! statically provisioned heap in a `no_std` kernel.

use core::{
    cmp,
    ptr::{self, NonNull},
};

use crate::kernel::sync::SpinLock;

pub const PAGE_SIZE: usize = 4096;
pub const DEFAULT_HEAP_BYTES: usize = 128 * 1024;
pub const MAX_ALLOCATION_RECORDS: usize = 512;

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
    offset: usize,
    size: usize,
    kind: AllocationKind,
    protection: MemoryProtection,
}

impl AllocationRecord {
    const fn new(
        offset: usize,
        size: usize,
        kind: AllocationKind,
        protection: MemoryProtection,
    ) -> Self {
        Self {
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
        if size == 0 {
            return None;
        }

        let align = core::mem::size_of::<usize>();
        let actual_size = self.align_up(size, align);
        let offset = self.reserve(actual_size, align)?;
        let record = AllocationRecord::new(
            offset,
            actual_size,
            AllocationKind::Heap,
            MemoryProtection::read_write(),
        );
        self.record_allocation(record)?;
        self.update_stats_on_alloc(actual_size);
        Some(unsafe { NonNull::new_unchecked(self.heap.as_mut_ptr().add(offset)) })
    }

    pub fn malloc_aligned(&mut self, size: usize, align: usize) -> Option<NonNull<u8>> {
        if size == 0 || align == 0 {
            return None;
        }

        let actual_align = align.max(core::mem::size_of::<usize>());
        let actual_size = self.align_up(size, core::mem::size_of::<usize>());
        if actual_size < size {
            return None;
        }

        let offset = self.reserve(actual_size, actual_align)?;
        let record = AllocationRecord::new(
            offset,
            actual_size,
            AllocationKind::Heap,
            MemoryProtection::read_write(),
        );
        self.record_allocation(record)?;
        self.update_stats_on_alloc(actual_size);
        Some(unsafe { NonNull::new_unchecked(self.heap.as_mut_ptr().add(offset)) })
    }

    pub fn realloc(&mut self, ptr: Option<NonNull<u8>>, new_size: usize) -> Option<NonNull<u8>> {
        match (ptr, new_size) {
            (None, 0) => None,
            (None, size) => self.malloc(size),
            (Some(p), 0) => {
                self.free(p);
                None
            }
            (Some(p), size) => {
                let base = self.heap.as_ptr() as usize;
                let addr = p.as_ptr() as usize;
                if addr < base || addr >= base + HEAP_SIZE {
                    return None;
                }
                let offset = addr - base;
                let idx = self.find_allocation_index(offset)?;
                let mut record = match self.allocations[idx] {
                    Some(r) => r,
                    None => return None,
                };
                if record.kind != AllocationKind::Heap {
                    return None;
                }

                let align = core::mem::size_of::<usize>();
                let aligned_new = self.align_up(size, align);
                if aligned_new < size {
                    return None;
                }

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
                let new_ptr = self.malloc(size)?;
                unsafe {
                    ptr::copy_nonoverlapping(p.as_ptr(), new_ptr.as_ptr(), copy_len);
                }
                self.free(p);
                Some(new_ptr)
            }
        }
    }

    pub fn free(&mut self, ptr: NonNull<u8>) -> bool {
        self.release(ptr, Some(AllocationKind::Heap), None)
    }

    pub fn mmap(&mut self, length: usize, protection: MemoryProtection) -> Option<MappedRegion> {
        if length == 0 {
            return None;
        }

        let align = PAGE_SIZE;
        let actual_size = self.align_up(length, PAGE_SIZE);
        let offset = self.reserve(actual_size, align)?;
        let record =
            AllocationRecord::new(offset, actual_size, AllocationKind::Mapping, protection);
        self.record_allocation(record)?;
        self.update_stats_on_alloc(actual_size);
        let ptr = unsafe { NonNull::new_unchecked(self.heap.as_mut_ptr().add(offset)) };
        Some(MappedRegion {
            ptr,
            length: actual_size,
            requested: length,
            protection,
            kind: AllocationKind::Mapping,
        })
    }

    pub fn munmap(&mut self, region: MappedRegion) -> bool {
        self.release(
            region.ptr,
            Some(AllocationKind::Mapping),
            Some(region.length),
        )
    }

    pub fn munmap_ptr(&mut self, ptr: NonNull<u8>, length: usize) -> bool {
        self.release(ptr, Some(AllocationKind::Mapping), Some(length))
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

        let aligned_offset = self.align_up(self.bump_offset, align);
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
                let aligned_start = self.align_up(region.offset, align);
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
        if let Some(record) = self.remove_allocation(offset, expected_kind, minimum_length) {
            self.insert_free_region(FreeRegion::new(record.offset, record.size));
            self.update_stats_on_free(record.size);
            true
        } else {
            false
        }
    }

    fn find_allocation_index(&self, offset: usize) -> Option<usize> {
        let mut idx = 0;
        while idx < MAX_AREAS {
            if let Some(record) = self.allocations[idx] {
                if record.offset == offset {
                    return Some(idx);
                }
            }
            idx += 1;
        }
        None
    }

    fn remove_allocation(
        &mut self,
        offset: usize,
        expected_kind: Option<AllocationKind>,
        minimum_length: Option<usize>,
    ) -> Option<AllocationRecord> {
        let mut idx = 0;
        while idx < MAX_AREAS {
            if let Some(record) = self.allocations[idx] {
                if record.offset == offset {
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

    fn align_up(&self, value: usize, align: usize) -> usize {
        if align == 0 {
            value
        } else {
            (value + (align - 1)) & !(align - 1)
        }
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
    MEMORY_MANAGER.lock().malloc(size)
}

pub fn malloc_aligned(size: usize, align: usize) -> Option<NonNull<u8>> {
    MEMORY_MANAGER.lock().malloc_aligned(size, align)
}

pub fn realloc(ptr: Option<NonNull<u8>>, new_size: usize) -> Option<NonNull<u8>> {
    MEMORY_MANAGER.lock().realloc(ptr, new_size)
}

pub fn free(ptr: NonNull<u8>) -> bool {
    MEMORY_MANAGER.lock().free(ptr)
}

pub fn mmap(length: usize, protection: MemoryProtection) -> Option<MappedRegion> {
    MEMORY_MANAGER.lock().mmap(length, protection)
}

pub fn munmap(region: MappedRegion) -> bool {
    MEMORY_MANAGER.lock().munmap(region)
}

pub fn munmap_ptr(ptr: NonNull<u8>, length: usize) -> bool {
    MEMORY_MANAGER.lock().munmap_ptr(ptr, length)
}

pub fn stats() -> AllocationStats {
    MEMORY_MANAGER.lock().statistics()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malloc_and_free_cycle() {
        let mut manager: MemoryManager<4096, 16> = MemoryManager::new();
        let ptr = manager.malloc(32).expect("allocation succeeds");
        assert!(manager.free(ptr));
        assert_eq!(manager.statistics().allocated_bytes, 0);
    }

    #[test]
    fn mmap_produces_page_aligned_region() {
        let mut manager: MemoryManager<8192, 32> = MemoryManager::new();
        let region = manager
            .mmap(4096, MemoryProtection::read_only())
            .expect("mapping succeeds");
        assert_eq!(region.length, 4096);
        assert_eq!((region.ptr.as_ptr() as usize) % PAGE_SIZE, 0);
        assert!(manager.munmap(region));
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
}
