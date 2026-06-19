//! Early kernel memory management.
//!
//! Mirage keeps the public `malloc`, `free`, and `mmap` APIs small, but the
//! backing store can now be promoted from the tiny static bootstrap heap to
//! page-backed virtual memory once x86_64 boot information has been ingested.

use core::{
    cmp,
    ptr::{self, NonNull},
};

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
use core::alloc::{GlobalAlloc, Layout};

use crate::arch::x86_64::{boot::BootInfo, paging};
use crate::kernel::process::ProcessId;
use crate::kernel::sync::SpinLock;

pub const PAGE_SIZE: usize = 4096;
pub const DEFAULT_HEAP_BYTES: usize = 128 * 1024;
pub const EARLY_HEAP_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_ALLOCATION_RECORDS: usize = 512;
pub const MAX_PHYSICAL_REGIONS: usize = 128;
pub const MAX_ADDRESS_SPACES: usize = 64;
pub const MAX_USER_MAPPINGS: usize = 2048;
pub const EARLY_HEAP_BASE: usize = 0xffff_9000_0000_0000;
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

pub mod frame_allocator;
pub mod heap;

pub use frame_allocator::{
    MemoryError, PhysFrame, PhysicalFrameAllocator, PhysicalMemoryStats, PhysicalRegion,
    PhysicalRegionKind,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackingStore {
    Static,
    Virtual {
        base: usize,
        capacity: usize,
        committed: usize,
        frames: usize,
    },
    Disabled,
}

impl BackingStore {
    fn capacity<const HEAP_SIZE: usize>(self) -> usize {
        match self {
            BackingStore::Static => HEAP_SIZE,
            BackingStore::Virtual { capacity, .. } => capacity,
            BackingStore::Disabled => 0,
        }
    }

    fn base<const HEAP_SIZE: usize>(self, static_heap: *const u8) -> usize {
        match self {
            BackingStore::Static => static_heap as usize,
            BackingStore::Virtual { base, .. } => base,
            BackingStore::Disabled => 0,
        }
    }
}

const fn align_up_u64(address: u64) -> u64 {
    address.saturating_add((PAGE_SIZE as u64) - 1) & !((PAGE_SIZE as u64) - 1)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeapStats {
    pub base: usize,
    pub end: usize,
    pub committed_bytes: usize,
    pub reserved_bytes: usize,
}

pub struct MemoryManager<const HEAP_SIZE: usize, const MAX_AREAS: usize> {
    heap: [u8; HEAP_SIZE],
    bump_offset: usize,
    allocations: [Option<AllocationRecord>; MAX_AREAS],
    free_regions: [Option<FreeRegion>; MAX_AREAS],
    allocated_bytes: usize,
    peak_bytes: usize,
    backing: BackingStore,
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
            backing: BackingStore::Static,
        }
    }

    pub fn promote_to_virtual_heap(
        &mut self,
        base: usize,
        capacity: usize,
        committed: usize,
        frames: usize,
    ) {
        if self.allocated_bytes != 0
            || self.bump_offset != 0
            || capacity == 0
            || committed > capacity
        {
            return;
        }
        self.backing = BackingStore::Virtual {
            base,
            capacity,
            committed,
            frames,
        };
    }

    pub fn disable_static_heap(&mut self) {
        if matches!(self.backing, BackingStore::Static) {
            self.backing = BackingStore::Disabled;
        }
    }

    fn ensure_backing(
        &mut self,
        offset: usize,
        size: usize,
        protection: MemoryProtection,
    ) -> Option<()> {
        match self.backing {
            BackingStore::Static => Some(()),
            BackingStore::Disabled => None,
            BackingStore::Virtual {
                base,
                capacity,
                committed,
                frames: _,
            } => {
                let required = offset.checked_add(size)?;
                if required > capacity {
                    return None;
                }
                let target = self.align_up(required, PAGE_SIZE)?;
                let layout =
                    heap::grow_committed(base, capacity, committed, target, protection).ok()?;
                self.backing = BackingStore::Virtual {
                    base,
                    capacity,
                    committed: layout.committed_bytes,
                    frames: layout.frame_count,
                };
                Some(())
            }
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
        if self
            .ensure_backing(offset, actual_size, MemoryProtection::read_write())
            .is_none()
        {
            self.insert_free_region(FreeRegion::new(offset, actual_size));
            return None;
        }
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
        Some(self.ptr_for_offset(offset))
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
        if self
            .ensure_backing(offset, actual_size, MemoryProtection::read_write())
            .is_none()
        {
            self.insert_free_region(FreeRegion::new(offset, actual_size));
            return None;
        }
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
        Some(self.ptr_for_offset(offset))
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
                let offset = self.offset_for_ptr(p)?;
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
        if self
            .ensure_backing(offset, actual_size, protection)
            .is_none()
        {
            self.insert_free_region(FreeRegion::new(offset, actual_size));
            return None;
        }
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
        let ptr = self.ptr_for_offset(offset);
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

    pub fn heap_statistics(&self) -> HeapStats {
        let base = self.base_address();
        let reserved = self.capacity();
        let committed = match self.backing {
            BackingStore::Static => reserved,
            BackingStore::Virtual { committed, .. } => committed,
            BackingStore::Disabled => 0,
        };
        HeapStats {
            base,
            end: base.saturating_add(reserved),
            committed_bytes: committed,
            reserved_bytes: reserved,
        }
    }

    fn base_address(&self) -> usize {
        self.backing.base::<HEAP_SIZE>(self.heap.as_ptr())
    }

    fn capacity(&self) -> usize {
        self.backing.capacity::<HEAP_SIZE>()
    }

    fn ptr_for_offset(&mut self, offset: usize) -> NonNull<u8> {
        unsafe { NonNull::new_unchecked((self.base_address() + offset) as *mut u8) }
    }

    fn offset_for_ptr(&self, ptr: NonNull<u8>) -> Option<usize> {
        let base = self.base_address();
        let addr = ptr.as_ptr() as usize;
        if addr < base || addr >= base.saturating_add(self.capacity()) {
            return None;
        }
        Some(addr - base)
    }

    fn reserve(&mut self, size: usize, align: usize) -> Option<usize> {
        if let Some(offset) = self.reserve_from_free_list(size, align) {
            return Some(offset);
        }

        let aligned_offset = self.aligned_heap_offset(self.bump_offset, align)?;
        let end = aligned_offset.checked_add(size)?;
        if end > self.capacity() {
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
        let Some(offset) = self.offset_for_ptr(ptr) else {
            return false;
        };
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

        let base_remainder = self.base_address() % align;
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
static PHYSICAL_ALLOCATOR: SpinLock<PhysicalFrameAllocator<MAX_PHYSICAL_REGIONS>> =
    SpinLock::new(PhysicalFrameAllocator::new());

/// Kernel-global allocator for Rust `alloc` types used by the boot skeleton.
///
/// This intentionally delegates to the existing Mirage heap path. Before boot
/// memory is ingested it uses the small static bootstrap heap; after
/// `initialize_from_boot_info` promotes the manager, allocations come from the
/// page-backed early virtual heap. This is a skeleton allocator, not a claim of
/// production-grade memory management.
#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
pub struct KernelGlobalAllocator;

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
unsafe impl GlobalAlloc for KernelGlobalAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return NonNull::<u8>::dangling().as_ptr();
        }

        malloc_aligned(layout.size(), layout.align())
            .map(NonNull::as_ptr)
            .unwrap_or(ptr::null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        if let Some(ptr) = NonNull::new(ptr) {
            let _ = free(ptr);
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, _layout: Layout, new_size: usize) -> *mut u8 {
        let Some(ptr) = NonNull::new(ptr) else {
            let layout = unsafe {
                Layout::from_size_align_unchecked(new_size, core::mem::align_of::<usize>())
            };
            return unsafe { self.alloc(layout) };
        };

        realloc(Some(ptr), new_size)
            .map(NonNull::as_ptr)
            .unwrap_or(ptr::null_mut())
    }
}

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
#[global_allocator]
static GLOBAL_ALLOCATOR: KernelGlobalAllocator = KernelGlobalAllocator;

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
#[alloc_error_handler]
fn alloc_error(_layout: Layout) -> ! {
    loop {
        crate::arch::x86_64::cpu_relax();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressSpace {
    pub owner: ProcessId,
    pub root: u64,
    pub references: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UserMappingRecord {
    owner: ProcessId,
    root: u64,
    user_start: u64,
    kernel_start: usize,
    length: usize,
    protection: MemoryProtection,
}

impl UserMappingRecord {
    fn contains(self, root: u64, address: u64, len: usize, write: bool) -> bool {
        if self.root != root || (write && !self.protection.write) {
            return false;
        }
        let end = match address.checked_add(len as u64) {
            Some(end) => end,
            None => return false,
        };
        address >= self.user_start && end <= self.user_start.saturating_add(self.length as u64)
    }
}

struct AddressSpaceTable {
    spaces: [Option<AddressSpace>; MAX_ADDRESS_SPACES],
    mappings: [Option<UserMappingRecord>; MAX_USER_MAPPINGS],
}

impl AddressSpaceTable {
    const fn new() -> Self {
        Self {
            spaces: [None; MAX_ADDRESS_SPACES],
            mappings: [None; MAX_USER_MAPPINGS],
        }
    }
}

static ADDRESS_SPACES: SpinLock<AddressSpaceTable> = SpinLock::new(AddressSpaceTable::new());

pub fn initialize_from_boot_info(boot_info: &BootInfo) {
    match PHYSICAL_ALLOCATOR.lock().ingest_boot_info(boot_info) {
        Ok(()) => {
            crate::kprintln!("physical frame allocator initialized");
        }
        Err(error) => {
            crate::kprintln!(
                "physical frame allocator initialization failed: {:?}",
                error
            );
            MEMORY_MANAGER.lock().disable_static_heap();
            return;
        }
    }

    match paging::enable_frame_backed_mapping(boot_info) {
        Ok(()) => {
            crate::kprintln!("frame-backed kernel mapper initialized");
        }
        Err(error) => {
            crate::kprintln!(
                "frame-backed kernel mapper initialization failed: {:?}",
                error
            );
            MEMORY_MANAGER.lock().disable_static_heap();
            return;
        }
    }

    match heap::initialize() {
        Ok(layout) => MEMORY_MANAGER.lock().promote_to_virtual_heap(
            layout.base,
            layout.reserved_bytes,
            layout.committed_bytes,
            layout.frame_count,
        ),
        Err(error) => {
            crate::kprintln!("kernel heap initialization failed: {:?}", error);
            MEMORY_MANAGER.lock().disable_static_heap();
        }
    }
}

pub fn physical_allocator_initialized() -> bool {
    PHYSICAL_ALLOCATOR.lock().initialized()
}

pub fn allocate_frame() -> Result<PhysFrame, MemoryError> {
    PHYSICAL_ALLOCATOR.lock().allocate_frame()
}

pub fn free_frame(frame: PhysFrame) -> Result<(), MemoryError> {
    PHYSICAL_ALLOCATOR.lock().free_frame(frame)
}

pub fn allocate_physical_frame() -> Option<u64> {
    allocate_frame().ok().map(PhysFrame::start_address)
}

pub fn deallocate_physical_frame(frame: u64) {
    if let Ok(frame) = PhysFrame::from_start_address(frame) {
        let _ = free_frame(frame);
    }
}

pub fn reserve_physical_range(start: u64, length: u64, kind: PhysicalRegionKind) {
    PHYSICAL_ALLOCATOR.lock().reserve_range(start, length, kind);
}

pub fn physical_stats() -> PhysicalMemoryStats {
    PHYSICAL_ALLOCATOR.lock().statistics()
}

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

pub fn create_user_address_space(owner: ProcessId) -> Option<u64> {
    let root = paging::create_user_address_space()?;
    let mut table = ADDRESS_SPACES.lock();
    let mut idx = 0usize;
    while idx < MAX_ADDRESS_SPACES {
        if table.spaces[idx].is_none() {
            table.spaces[idx] = Some(AddressSpace {
                owner,
                root,
                references: 1,
            });
            return Some(root);
        }
        idx += 1;
    }
    drop(table);
    paging::destroy_user_address_space(root);
    None
}

pub fn share_user_address_space(root: u64) -> Option<u64> {
    if root == 0 {
        return None;
    }
    let mut table = ADDRESS_SPACES.lock();
    for slot in table.spaces.iter_mut() {
        if let Some(space) = slot.as_mut() {
            if space.root == root {
                space.references = space.references.saturating_add(1);
                return Some(root);
            }
        }
    }
    None
}

pub fn clone_user_address_space(owner: ProcessId, parent_root: u64) -> Option<u64> {
    if parent_root == 0 {
        return create_user_address_space(owner);
    }
    let child_root = create_user_address_space(owner)?;
    let mappings = ADDRESS_SPACES.lock().mappings;
    let mut idx = 0usize;
    while idx < MAX_USER_MAPPINGS {
        if let Some(mapping) = mappings[idx] {
            if mapping.root == parent_root {
                let child = mmap_user_fixed(
                    owner,
                    child_root,
                    mapping.user_start,
                    mapping.length,
                    mapping.protection,
                )?;
                unsafe {
                    ptr::copy_nonoverlapping(
                        mapping.kernel_start as *const u8,
                        child.as_ptr(),
                        mapping.length,
                    );
                }
            }
        }
        idx += 1;
    }
    Some(child_root)
}

pub fn destroy_user_address_space(root: u64) {
    if root == 0 {
        return;
    }
    let mut should_destroy = false;
    {
        let mut table = ADDRESS_SPACES.lock();
        let mut idx = 0usize;
        while idx < MAX_ADDRESS_SPACES {
            if let Some(mut space) = table.spaces[idx] {
                if space.root == root {
                    if space.references > 1 {
                        space.references -= 1;
                        table.spaces[idx] = Some(space);
                        return;
                    }
                    table.spaces[idx] = None;
                    should_destroy = true;
                    break;
                }
            }
            idx += 1;
        }
        idx = 0;
        while idx < MAX_USER_MAPPINGS {
            if let Some(mapping) = table.mappings[idx] {
                if mapping.root == root {
                    table.mappings[idx] = None;
                    if let Some(ptr) = NonNull::new(mapping.kernel_start as *mut u8) {
                        let _ = MEMORY_MANAGER.lock().munmap_ptr_for(
                            mapping.owner,
                            ptr,
                            mapping.length,
                        );
                    }
                }
            }
            idx += 1;
        }
    }
    if should_destroy {
        paging::destroy_user_address_space(root);
    }
}

pub fn mmap_user_fixed(
    owner: ProcessId,
    address_space_root: u64,
    virtual_address: u64,
    length: usize,
    protection: MemoryProtection,
) -> Option<MappedRegion> {
    if address_space_root == 0
        || length == 0
        || virtual_address & ((PAGE_SIZE as u64) - 1) != 0
        || virtual_address.checked_add(length as u64)? > 0x0000_8000_0000_0000
    {
        return None;
    }
    let actual_size = align_up_u64(length as u64) as usize;
    let region = mmap_for(owner, actual_size, protection)?;
    let mut offset = 0usize;
    while offset < actual_size {
        let kernel_va = region.as_ptr() as u64 + offset as u64;
        let physical = paging::translate_kernel_address(kernel_va)
            .unwrap_or_else(|| paging::active_translator().physical_for_virtual(kernel_va));
        if paging::map_user_page(
            address_space_root,
            virtual_address + offset as u64,
            physical,
            protection,
        )
        .is_none()
        {
            let _ = munmap(region);
            return None;
        }
        offset += PAGE_SIZE;
    }
    let mut table = ADDRESS_SPACES.lock();
    let mut idx = 0usize;
    while idx < MAX_USER_MAPPINGS {
        if table.mappings[idx].is_none() {
            table.mappings[idx] = Some(UserMappingRecord {
                owner,
                root: address_space_root,
                user_start: virtual_address,
                kernel_start: region.as_ptr() as usize,
                length: actual_size,
                protection,
            });
            return Some(region);
        }
        idx += 1;
    }
    let _ = munmap(region);
    None
}

pub fn find_user_mapping(
    address_space_root: u64,
    user_address: u64,
    length: usize,
    write: bool,
) -> Option<MappedRegion> {
    let table = ADDRESS_SPACES.lock();
    let mut idx = 0usize;
    while idx < MAX_USER_MAPPINGS {
        if let Some(mapping) = table.mappings[idx] {
            if mapping.contains(address_space_root, user_address, length, write) {
                return Some(MappedRegion {
                    owner: mapping.owner,
                    ptr: NonNull::new(mapping.kernel_start as *mut u8)?,
                    length: mapping.length,
                    requested: mapping.length,
                    protection: mapping.protection,
                    kind: AllocationKind::Mapping,
                });
            }
        }
        idx += 1;
    }
    None
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
    let mut roots = [0u64; MAX_ADDRESS_SPACES];
    let mut count = 0usize;
    {
        let table = ADDRESS_SPACES.lock();
        let mut idx = 0usize;
        while idx < MAX_ADDRESS_SPACES {
            if let Some(space) = table.spaces[idx] {
                if space.owner == owner && count < MAX_ADDRESS_SPACES {
                    roots[count] = space.root;
                    count += 1;
                }
            }
            idx += 1;
        }
    }
    let mut idx = 0usize;
    while idx < count {
        destroy_user_address_space(roots[idx]);
        idx += 1;
    }
    MEMORY_MANAGER.lock().release_process(owner);
}

pub fn active_translated_slice(
    root: u64,
    ptr: u64,
    len: usize,
    write: bool,
) -> Option<NonNull<u8>> {
    if len == 0 {
        return NonNull::new(core::ptr::NonNull::<u8>::dangling().as_ptr());
    }
    let table = ADDRESS_SPACES.lock();
    let mut idx = 0usize;
    while idx < MAX_USER_MAPPINGS {
        if let Some(mapping) = table.mappings[idx] {
            if mapping.contains(root, ptr, len, write) {
                let offset = ptr.saturating_sub(mapping.user_start) as usize;
                return NonNull::new((mapping.kernel_start + offset) as *mut u8);
            }
        }
        idx += 1;
    }
    None
}

pub fn validate_user_range(root: u64, ptr: u64, len: usize, write: bool) -> bool {
    if len == 0 {
        return true;
    }
    active_translated_slice(root, ptr, len, write).is_some()
}

pub fn copy_from_user(root: u64, ptr: u64, out: &mut [u8]) -> bool {
    let src = match active_translated_slice(root, ptr, out.len(), false) {
        Some(src) => src,
        None => return false,
    };
    unsafe { ptr::copy_nonoverlapping(src.as_ptr(), out.as_mut_ptr(), out.len()) };
    true
}

pub fn copy_to_user(root: u64, ptr: u64, input: &[u8]) -> bool {
    let dst = match active_translated_slice(root, ptr, input.len(), true) {
        Some(dst) => dst,
        None => return false,
    };
    unsafe { ptr::copy_nonoverlapping(input.as_ptr(), dst.as_ptr(), input.len()) };
    true
}

pub fn stats() -> AllocationStats {
    MEMORY_MANAGER.lock().statistics()
}

pub fn heap_stats() -> HeapStats {
    MEMORY_MANAGER.lock().heap_statistics()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::boxed::Box;
    use std::vec::Vec;

    fn offset_of<const HEAP_SIZE: usize, const MAX_AREAS: usize>(
        manager: &MemoryManager<HEAP_SIZE, MAX_AREAS>,
        ptr: NonNull<u8>,
    ) -> usize {
        ptr.as_ptr() as usize - manager.heap.as_ptr() as usize
    }

    #[test]
    fn physical_frame_allocation_uses_only_mock_usable_regions() {
        let mut allocator: PhysicalFrameAllocator<4> = PhysicalFrameAllocator::new();
        assert!(allocator.add_region(PhysicalRegion::new(
            0x1000,
            PAGE_SIZE as u64,
            PhysicalRegionKind::Reserved,
        )));
        assert!(allocator.add_region(PhysicalRegion::new(
            0x4000,
            (PAGE_SIZE * 2) as u64,
            PhysicalRegionKind::Usable,
        )));
        let mut metadata = [0u8; 8];
        allocator
            .initialize_with_metadata(&mut metadata)
            .expect("metadata initializes");

        assert_eq!(allocator.allocate_frame().unwrap().start_address(), 0x4000);
        assert_eq!(allocator.allocate_frame().unwrap().start_address(), 0x5000);
        assert_eq!(allocator.allocate_frame(), Err(MemoryError::OutOfMemory));
        assert_eq!(allocator.statistics().allocated_frames, 2);

        let frame = PhysFrame::from_start_address(0x4000).unwrap();
        assert_eq!(allocator.free_frame(frame), Ok(()));
        assert_eq!(allocator.free_frame(frame), Err(MemoryError::DoubleFree));
        assert_eq!(allocator.allocate_frame().unwrap().start_address(), 0x4000);
    }

    #[test]
    fn heap_allocation_via_malloc_path_is_host_isolated() {
        let mut manager: MemoryManager<4096, 16> = MemoryManager::new();
        let ptr = manager.malloc(64).expect("allocation succeeds");

        unsafe {
            ptr::write_bytes(ptr.as_ptr(), 0xab, 64);
            for index in 0..64 {
                assert_eq!(ptr.as_ptr().add(index).read(), 0xab);
            }
        }

        assert!(manager.statistics().allocated_bytes >= 64);
        assert!(manager.free(ptr));
        assert_eq!(manager.statistics().allocated_bytes, 0);
    }

    #[test]
    fn box_allocation_smoke_test_stays_on_host_allocator() {
        // Host tests intentionally do not install the kernel global allocator;
        // this verifies `alloc`-style usage without pretending QEMU boot memory
        // was initialized by the unit-test process.
        let boxed = Box::new(0x4d4952414745_u64);

        assert_eq!(*boxed, 0x4d4952414745_u64);
    }

    #[test]
    fn vec_allocation_smoke_test_stays_on_host_allocator() {
        // This is an allocation smoke test for the skeleton API surface, not a
        // production heap stress test.
        let mut values = Vec::new();
        values.extend_from_slice(&[1, 2, 3, 5, 8, 13]);

        assert_eq!(values.len(), 6);
        assert_eq!(values.iter().copied().sum::<u32>(), 32);
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
