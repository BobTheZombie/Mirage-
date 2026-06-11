//! 4 KiB physical-frame ownership database for early kernel memory.
//!
//! The allocator keeps Limine's original memory-map accounting separate from
//! the live ownership state. Only `Usable` frames enter the free pool; all other
//! Limine kinds are represented as reserved, ACPI, MMIO, kernel/module, or
//! bootloader-reclaimable ownership until a later supervisor policy explicitly
//! reclaims them.

use core::{cmp, mem, ptr};

use crate::arch::x86_64::{
    boot::{BootInfo, MemoryRegionKind},
    paging,
};
use crate::kernel::memory::PAGE_SIZE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysFrame {
    start: u64,
}

impl PhysFrame {
    pub const fn containing_address(address: u64) -> Self {
        Self {
            start: align_down_u64(address),
        }
    }

    pub const fn from_start_address(start: u64) -> Result<Self, MemoryError> {
        if start & (PAGE_SIZE as u64 - 1) == 0 {
            Ok(Self { start })
        } else {
            Err(MemoryError::UnalignedFrame)
        }
    }

    pub const fn start_address(self) -> u64 {
        self.start
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryError {
    NotInitialized,
    OutOfMemory,
    MetadataUnavailable,
    MetadataTooSmall,
    InvalidFrame,
    DoubleFree,
    UnalignedFrame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalRegionKind {
    Usable,
    Reserved,
    Acpi,
    Mmio,
    Kernel,
    Module,
    BootloaderReclaimable,
    AllocatorMetadata,
}

impl PhysicalRegionKind {
    const fn from_boot(kind: MemoryRegionKind) -> Self {
        match kind {
            MemoryRegionKind::Usable => Self::Usable,
            MemoryRegionKind::AcpiReclaimable | MemoryRegionKind::AcpiNvs => Self::Acpi,
            MemoryRegionKind::Framebuffer => Self::Mmio,
            MemoryRegionKind::KernelAndModules => Self::Kernel,
            MemoryRegionKind::BootloaderReclaimable => Self::BootloaderReclaimable,
            MemoryRegionKind::Reserved
            | MemoryRegionKind::BadMemory
            | MemoryRegionKind::Unknown(_) => Self::Reserved,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalRegion {
    pub start: u64,
    pub length: u64,
    pub kind: PhysicalRegionKind,
}

impl PhysicalRegion {
    pub const fn new(start: u64, length: u64, kind: PhysicalRegionKind) -> Self {
        Self {
            start,
            length,
            kind,
        }
    }

    pub const fn end(self) -> u64 {
        self.start.saturating_add(self.length)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalMemoryStats {
    /// Original bytes described by the boot memory map, before allocator reservations.
    pub total_memory_map_bytes: u64,
    /// Backwards-compatible alias for callers that still display `total_bytes`.
    pub total_bytes: u64,
    /// Original Limine `Usable` bytes. This is not decremented as frames are allocated.
    pub total_usable_bytes: u64,
    /// Boot memory-map usable bytes. Kept separate from live free-frame counts.
    pub usable_bytes: u64,
    /// Original non-usable/reserved bytes plus explicit kernel/module/MMIO/metadata reservations.
    pub reserved_bytes: u64,
    pub bootloader_reclaimable_bytes: u64,
    pub acpi_bytes: u64,
    /// Live MMIO and framebuffer reservation bytes.
    pub mmio_framebuffer_bytes: u64,
    /// Backwards-compatible MMIO byte count.
    pub mmio_bytes: u64,
    /// Live kernel image plus boot module reservation bytes.
    pub kernel_module_bytes: u64,
    pub kernel_bytes: u64,
    pub module_bytes: u64,
    pub total_frame_count: usize,
    pub free_frame_count: usize,
    pub used_frame_count: usize,
    pub frame_count: usize,
    pub free_frames: usize,
    pub used_frames: usize,
    /// Backwards-compatible allocated-frame count.
    pub allocated_frames: usize,
    pub metadata_bytes: usize,
    pub metadata_physical_start: u64,
    pub metadata_physical_end: u64,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameState {
    Invalid = 0,
    Free = 1,
    Allocated = 2,
    Reserved = 3,
    BootloaderReclaimable = 4,
}

pub struct PhysicalFrameAllocator<const MAX_REGIONS: usize> {
    regions: [Option<PhysicalRegion>; MAX_REGIONS],
    metadata: *mut FrameState,
    frame_count: usize,
    metadata_bytes: usize,
    metadata_physical_start: u64,
    free_frames: usize,
    used_frames: usize,
    stats: PhysicalMemoryStats,
    initialized: bool,
}

unsafe impl<const MAX_REGIONS: usize> Send for PhysicalFrameAllocator<MAX_REGIONS> {}

impl<const MAX_REGIONS: usize> PhysicalFrameAllocator<MAX_REGIONS> {
    pub const fn new() -> Self {
        Self {
            regions: [None; MAX_REGIONS],
            metadata: ptr::null_mut(),
            frame_count: 0,
            metadata_bytes: 0,
            metadata_physical_start: 0,
            free_frames: 0,
            used_frames: 0,
            stats: PhysicalMemoryStats {
                total_memory_map_bytes: 0,
                total_bytes: 0,
                total_usable_bytes: 0,
                usable_bytes: 0,
                reserved_bytes: 0,
                bootloader_reclaimable_bytes: 0,
                acpi_bytes: 0,
                mmio_framebuffer_bytes: 0,
                mmio_bytes: 0,
                kernel_module_bytes: 0,
                kernel_bytes: 0,
                module_bytes: 0,
                total_frame_count: 0,
                free_frame_count: 0,
                used_frame_count: 0,
                frame_count: 0,
                free_frames: 0,
                used_frames: 0,
                allocated_frames: 0,
                metadata_bytes: 0,
                metadata_physical_start: 0,
                metadata_physical_end: 0,
            },
            initialized: false,
        }
    }

    pub fn ingest_boot_info(&mut self, boot_info: &BootInfo) -> Result<(), MemoryError> {
        self.reset();
        let translator = paging::AddressTranslator::new(boot_info);
        let mut max_end = 0u64;

        if let Some(map) = boot_info.memory_map {
            let mut index = 0;
            while index < map.len() {
                if let Some(entry) = map.entry(index) {
                    let raw_start = entry.base.0;
                    let raw_end = entry.base.0.saturating_add(entry.length);
                    let kind = PhysicalRegionKind::from_boot(entry.kind);
                    self.account_original_region(kind, entry.length);

                    let (start, end) = if kind == PhysicalRegionKind::Usable {
                        (align_up_u64(raw_start), align_down_u64(raw_end))
                    } else {
                        (align_down_u64(raw_start), align_up_u64(raw_end))
                    };
                    if start < end {
                        self.add_region(PhysicalRegion::new(
                            start,
                            end.saturating_sub(start),
                            kind,
                        ));
                    }
                    max_end = cmp::max(max_end, align_up_u64(raw_end));
                }
                index += 1;
            }
        }

        self.reserve_boot_owned_ranges(boot_info, &translator);
        self.frame_count =
            frame_index(max_end).saturating_add(usize::from(max_end % PAGE_SIZE as u64 != 0));
        self.metadata_bytes = self
            .frame_count
            .saturating_mul(mem::size_of::<FrameState>());
        let metadata_physical = self
            .find_metadata_range(self.metadata_bytes as u64)
            .ok_or(MemoryError::MetadataUnavailable)?;
        self.reserve_range(
            metadata_physical,
            self.metadata_bytes as u64,
            PhysicalRegionKind::AllocatorMetadata,
        );

        self.metadata_physical_start = metadata_physical;
        self.metadata = translator.virtual_for_physical(metadata_physical) as *mut FrameState;
        if self.metadata.is_null() {
            return Err(MemoryError::MetadataUnavailable);
        }

        unsafe { self.initialize_database() };
        self.initialized = true;
        self.refresh_stats();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn initialize_with_metadata(
        &mut self,
        metadata: &mut [u8],
    ) -> Result<(), MemoryError> {
        let mut max_end = 0u64;
        let mut idx = 0;
        while idx < MAX_REGIONS {
            if let Some(region) = self.regions[idx] {
                self.account_original_region(region.kind, region.length);
                max_end = cmp::max(max_end, region.end());
            }
            idx += 1;
        }
        self.frame_count =
            frame_index(max_end).saturating_add(usize::from(max_end % PAGE_SIZE as u64 != 0));
        self.metadata_bytes = self
            .frame_count
            .saturating_mul(mem::size_of::<FrameState>());
        if metadata.len() < self.metadata_bytes {
            return Err(MemoryError::MetadataTooSmall);
        }
        self.metadata = metadata.as_mut_ptr() as *mut FrameState;
        unsafe { self.initialize_database() };
        self.initialized = true;
        self.refresh_stats();
        Ok(())
    }

    pub fn allocate_frame(&mut self) -> Result<PhysFrame, MemoryError> {
        if !self.initialized || self.metadata.is_null() {
            return Err(MemoryError::NotInitialized);
        }

        let mut index = 0usize;
        while index < self.frame_count {
            if unsafe { *self.metadata.add(index) } == FrameState::Free {
                unsafe { *self.metadata.add(index) = FrameState::Allocated };
                self.free_frames = self.free_frames.saturating_sub(1);
                self.used_frames = self.used_frames.saturating_add(1);
                self.refresh_stats();
                return Ok(PhysFrame {
                    start: (index as u64).saturating_mul(PAGE_SIZE as u64),
                });
            }
            index += 1;
        }

        Err(MemoryError::OutOfMemory)
    }

    pub fn free_frame(&mut self, frame: PhysFrame) -> Result<(), MemoryError> {
        if !self.initialized || self.metadata.is_null() {
            return Err(MemoryError::NotInitialized);
        }
        if frame.start_address() & (PAGE_SIZE as u64 - 1) != 0 {
            return Err(MemoryError::UnalignedFrame);
        }
        let index = frame_index(frame.start_address());
        if index >= self.frame_count {
            return Err(MemoryError::InvalidFrame);
        }

        match unsafe { *self.metadata.add(index) } {
            FrameState::Allocated => {
                unsafe { *self.metadata.add(index) = FrameState::Free };
                self.free_frames = self.free_frames.saturating_add(1);
                self.used_frames = self.used_frames.saturating_sub(1);
                self.refresh_stats();
                Ok(())
            }
            FrameState::Free => Err(MemoryError::DoubleFree),
            FrameState::Invalid | FrameState::Reserved | FrameState::BootloaderReclaimable => {
                Err(MemoryError::InvalidFrame)
            }
        }
    }

    pub fn reserve_range(&mut self, start: u64, length: u64, kind: PhysicalRegionKind) {
        if length == 0 {
            return;
        }
        let reserve_start = align_down_u64(start);
        let reserve_end = align_up_u64(start.saturating_add(length));
        let mut idx = 0;
        while idx < MAX_REGIONS {
            if let Some(region) = self.regions[idx] {
                if ranges_overlap(region.start, region.end(), reserve_start, reserve_end) {
                    self.regions[idx] = None;
                    if region.start < reserve_start {
                        self.add_region(PhysicalRegion::new(
                            region.start,
                            reserve_start.saturating_sub(region.start),
                            region.kind,
                        ));
                    }
                    if reserve_end < region.end() {
                        self.add_region(PhysicalRegion::new(
                            reserve_end,
                            region.end().saturating_sub(reserve_end),
                            region.kind,
                        ));
                    }
                }
            }
            idx += 1;
        }
        self.add_region(PhysicalRegion::new(
            reserve_start,
            reserve_end.saturating_sub(reserve_start),
            kind,
        ));

        if self.initialized && !self.metadata.is_null() {
            self.mark_frame_range(reserve_start, reserve_end, state_for_reserved_kind(kind));
            self.refresh_stats();
        }
    }

    pub fn initialized(&self) -> bool {
        self.initialized
    }

    pub fn statistics(&self) -> PhysicalMemoryStats {
        self.stats
    }

    pub(crate) fn add_region(&mut self, region: PhysicalRegion) -> bool {
        if region.length == 0 {
            return true;
        }
        let mut idx = 0;
        while idx < MAX_REGIONS {
            if let Some(existing) = self.regions[idx] {
                if existing.kind == region.kind && existing.end() == region.start {
                    self.regions[idx] = Some(PhysicalRegion::new(
                        existing.start,
                        existing.length.saturating_add(region.length),
                        existing.kind,
                    ));
                    return true;
                }
                if existing.kind == region.kind && region.end() == existing.start {
                    self.regions[idx] = Some(PhysicalRegion::new(
                        region.start,
                        existing.length.saturating_add(region.length),
                        existing.kind,
                    ));
                    return true;
                }
            }
            idx += 1;
        }
        idx = 0;
        while idx < MAX_REGIONS {
            if self.regions[idx].is_none() {
                self.regions[idx] = Some(region);
                return true;
            }
            idx += 1;
        }
        false
    }

    fn reset(&mut self) {
        self.regions = [None; MAX_REGIONS];
        self.metadata = ptr::null_mut();
        self.frame_count = 0;
        self.metadata_bytes = 0;
        self.metadata_physical_start = 0;
        self.free_frames = 0;
        self.used_frames = 0;
        self.stats = Self::new().stats;
        self.initialized = false;
    }

    fn account_original_region(&mut self, kind: PhysicalRegionKind, length: u64) {
        self.stats.total_memory_map_bytes =
            self.stats.total_memory_map_bytes.saturating_add(length);
        self.stats.total_bytes = self.stats.total_memory_map_bytes;
        match kind {
            PhysicalRegionKind::Usable => {
                self.stats.total_usable_bytes =
                    self.stats.total_usable_bytes.saturating_add(length);
                self.stats.usable_bytes = self.stats.total_usable_bytes;
            }
            PhysicalRegionKind::Acpi => {
                self.stats.acpi_bytes = self.stats.acpi_bytes.saturating_add(length)
            }
            PhysicalRegionKind::Mmio => {
                self.stats.mmio_bytes = self.stats.mmio_bytes.saturating_add(length)
            }
            PhysicalRegionKind::Kernel => {
                self.stats.kernel_bytes = self.stats.kernel_bytes.saturating_add(length)
            }
            PhysicalRegionKind::Module => {
                self.stats.module_bytes = self.stats.module_bytes.saturating_add(length)
            }
            PhysicalRegionKind::BootloaderReclaimable => {
                self.stats.bootloader_reclaimable_bytes = self
                    .stats
                    .bootloader_reclaimable_bytes
                    .saturating_add(length)
            }
            PhysicalRegionKind::Reserved | PhysicalRegionKind::AllocatorMetadata => {
                self.stats.reserved_bytes = self.stats.reserved_bytes.saturating_add(length)
            }
        }
    }

    fn reserve_boot_owned_ranges(
        &mut self,
        boot_info: &BootInfo,
        translator: &paging::AddressTranslator,
    ) {
        if let Some(load) = boot_info.kernel.load_range {
            self.reserve_range(
                load.physical_start.0,
                load.length,
                PhysicalRegionKind::Kernel,
            );
        } else {
            let sections = boot_info.kernel.sections;
            self.reserve_range(
                translator.physical_for_virtual(sections.kernel.start.0),
                sections.kernel.length(),
                PhysicalRegionKind::Kernel,
            );
        }

        let mut module_index = 0;
        while module_index < boot_info.modules.len() {
            if let Some(module) = boot_info.modules.module(module_index) {
                self.reserve_range(
                    translator.physical_for_virtual(module.base.0),
                    module.size,
                    PhysicalRegionKind::Module,
                );
            }
            module_index += 1;
        }

        if let Some(framebuffer) = boot_info.framebuffer {
            self.reserve_range(
                translator.physical_for_virtual(framebuffer.address.0),
                framebuffer.pitch.saturating_mul(framebuffer.height),
                PhysicalRegionKind::Mmio,
            );
        }

        if let Some(rsdp) = boot_info.rsdp {
            self.reserve_range(rsdp.0, PAGE_SIZE as u64, PhysicalRegionKind::Acpi);
        }

        let (tables_start, tables_len) = paging::page_table_pool_range();
        self.reserve_range(
            translator.physical_for_virtual(tables_start as u64),
            tables_len as u64,
            PhysicalRegionKind::Kernel,
        );

        let stack_probe = 0u8;
        let stack_virtual = core::ptr::addr_of!(stack_probe) as u64;
        let stack_physical = translator.physical_for_virtual(stack_virtual);
        self.reserve_range(
            stack_physical.saturating_sub(64 * 1024),
            64 * 1024,
            PhysicalRegionKind::Kernel,
        );
    }

    fn find_metadata_range(&self, bytes: u64) -> Option<u64> {
        if bytes == 0 {
            return None;
        }
        let required = align_up_u64(bytes);
        let mut idx = 0;
        while idx < MAX_REGIONS {
            if let Some(region) = self.regions[idx] {
                if region.kind == PhysicalRegionKind::Usable {
                    let start = align_up_u64(region.start);
                    let end = align_down_u64(region.end());
                    if start < end && end.saturating_sub(start) >= required {
                        return Some(start);
                    }
                }
            }
            idx += 1;
        }
        None
    }

    unsafe fn initialize_database(&mut self) {
        let mut index = 0usize;
        while index < self.frame_count {
            *self.metadata.add(index) = FrameState::Invalid;
            index += 1;
        }

        let mut idx = 0;
        while idx < MAX_REGIONS {
            if let Some(region) = self.regions[idx] {
                let state = match region.kind {
                    PhysicalRegionKind::Usable => FrameState::Free,
                    PhysicalRegionKind::BootloaderReclaimable => FrameState::BootloaderReclaimable,
                    _ => FrameState::Reserved,
                };
                self.mark_frame_range(region.start, region.end(), state);
            }
            idx += 1;
        }
    }

    fn mark_frame_range(&mut self, start: u64, end: u64, state: FrameState) {
        let mut index = frame_index(align_down_u64(start));
        let end_index = frame_index(align_up_u64(end));
        while index < end_index && index < self.frame_count {
            let old = unsafe { *self.metadata.add(index) };
            if old == FrameState::Free && state != FrameState::Free {
                self.free_frames = self.free_frames.saturating_sub(1);
            }
            if old == FrameState::Allocated && state != FrameState::Allocated {
                self.used_frames = self.used_frames.saturating_sub(1);
            }
            unsafe { *self.metadata.add(index) = state };
            index += 1;
        }
    }

    fn refresh_stats(&mut self) {
        let mut free = 0usize;
        let mut used = 0usize;
        let mut idx = 0usize;
        while !self.metadata.is_null() && idx < self.frame_count {
            match unsafe { *self.metadata.add(idx) } {
                FrameState::Free => free = free.saturating_add(1),
                FrameState::Allocated => used = used.saturating_add(1),
                _ => {}
            }
            idx += 1;
        }
        self.free_frames = free;
        self.used_frames = used;
        self.stats.total_frame_count = self.frame_count;
        self.stats.free_frame_count = self.free_frames;
        self.stats.used_frame_count = self.used_frames;
        self.stats.frame_count = self.frame_count;
        self.stats.free_frames = self.free_frames;
        self.stats.used_frames = self.used_frames;
        self.stats.allocated_frames = self.used_frames;
        self.stats.metadata_bytes = self.metadata_bytes;
        self.stats.metadata_physical_start = self.metadata_physical_start;
        self.stats.metadata_physical_end = self
            .metadata_physical_start
            .saturating_add(self.metadata_bytes as u64);
        self.stats.reserved_bytes = self
            .live_bytes_for(PhysicalRegionKind::Reserved)
            .saturating_add(self.live_bytes_for(PhysicalRegionKind::AllocatorMetadata));
        self.stats.kernel_bytes = self.live_bytes_for(PhysicalRegionKind::Kernel);
        self.stats.module_bytes = self.live_bytes_for(PhysicalRegionKind::Module);
        self.stats.kernel_module_bytes = self
            .stats
            .kernel_bytes
            .saturating_add(self.stats.module_bytes);
        self.stats.mmio_bytes = self.live_bytes_for(PhysicalRegionKind::Mmio);
        self.stats.mmio_framebuffer_bytes = self.stats.mmio_bytes;
        self.stats.acpi_bytes = self.live_bytes_for(PhysicalRegionKind::Acpi);
        self.stats.bootloader_reclaimable_bytes =
            self.live_bytes_for(PhysicalRegionKind::BootloaderReclaimable);
    }

    fn live_bytes_for(&self, kind: PhysicalRegionKind) -> u64 {
        let mut total = 0u64;
        let mut idx = 0;
        while idx < MAX_REGIONS {
            if let Some(region) = self.regions[idx] {
                if region.kind == kind {
                    total = total.saturating_add(region.length);
                }
            }
            idx += 1;
        }
        total
    }
}

const fn state_for_reserved_kind(kind: PhysicalRegionKind) -> FrameState {
    match kind {
        PhysicalRegionKind::Usable => FrameState::Free,
        PhysicalRegionKind::BootloaderReclaimable => FrameState::BootloaderReclaimable,
        _ => FrameState::Reserved,
    }
}

const fn frame_index(address: u64) -> usize {
    (address / PAGE_SIZE as u64) as usize
}

pub const fn align_down_u64(value: u64) -> u64 {
    value & !(PAGE_SIZE as u64 - 1)
}

pub const fn align_up_u64(value: u64) -> u64 {
    let mask = PAGE_SIZE as u64 - 1;
    value.saturating_add(mask) & !mask
}

fn ranges_overlap(a_start: u64, a_end: u64, b_start: u64, b_end: u64) -> bool {
    a_start < b_end && b_start < a_end
}

// TODO(huge-pages): add 2 MiB/1 GiB frame ownership only after the 4 KiB
// frame database is exercised by boot, allocation, free, and reservation paths.
