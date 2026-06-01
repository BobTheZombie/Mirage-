//! Early x86_64 page table construction from bootloader memory information.

use core::sync::atomic::{AtomicBool, Ordering};

use super::boot::{BootInfo, MemoryRegionKind};
use crate::kernel::memory::MemoryProtection;

const PAGE_SIZE: u64 = 4096;
const ENTRY_COUNT: usize = 512;
const MAX_TABLES: usize = 128;
const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const NO_EXECUTE: u64 = 1 << 63;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const DEFAULT_IDENTITY_LIMIT: u64 = 16 * 1024 * 1024;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static mut TABLES: PageTablePool = PageTablePool::new();
static mut ACTIVE_TRANSLATOR: AddressTranslator = AddressTranslator::identity();
static mut ACTIVE_PML4: *mut PageTable = core::ptr::null_mut();

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct PageTable {
    entries: [u64; ENTRY_COUNT],
}

impl PageTable {
    const fn empty() -> Self {
        Self {
            entries: [0; ENTRY_COUNT],
        }
    }
}

struct PageTablePool {
    tables: [PageTable; MAX_TABLES],
    next: usize,
}

impl PageTablePool {
    const fn new() -> Self {
        Self {
            tables: [PageTable::empty(); MAX_TABLES],
            next: 1,
        }
    }

    unsafe fn reset(&mut self) {
        for table in self.tables.iter_mut() {
            table.entries.fill(0);
        }
        self.next = 1;
    }

    unsafe fn pml4(&mut self) -> *mut PageTable {
        &mut self.tables[0]
    }

    unsafe fn allocate(&mut self) -> Option<*mut PageTable> {
        if self.next >= MAX_TABLES {
            return None;
        }
        let table = &mut self.tables[self.next] as *mut PageTable;
        self.next += 1;
        (*table).entries.fill(0);
        Some(table)
    }

    unsafe fn physical_from_entry(entry: u64) -> u64 {
        entry & ADDRESS_MASK
    }
}

#[derive(Clone, Copy)]
pub struct AddressTranslator {
    hhdm_offset: Option<u64>,
    kernel_physical_start: u64,
    kernel_virtual_start: u64,
    kernel_length: u64,
}

impl AddressTranslator {
    pub const fn identity() -> Self {
        Self {
            hhdm_offset: None,
            kernel_physical_start: 0,
            kernel_virtual_start: 0,
            kernel_length: 0,
        }
    }

    pub fn new(boot_info: &BootInfo) -> Self {
        let load = boot_info.kernel.load_range;
        Self {
            hhdm_offset: boot_info.hhdm_offset,
            kernel_physical_start: load.map(|range| range.physical_start.0).unwrap_or(0),
            kernel_virtual_start: load.map(|range| range.virtual_start.0).unwrap_or(0),
            kernel_length: load.map(|range| range.length).unwrap_or(0),
        }
    }

    pub fn physical_for_virtual(self, virtual_address: u64) -> u64 {
        if self.kernel_length != 0
            && virtual_address >= self.kernel_virtual_start
            && virtual_address < self.kernel_virtual_start.saturating_add(self.kernel_length)
        {
            return self
                .kernel_physical_start
                .saturating_add(virtual_address.saturating_sub(self.kernel_virtual_start));
        }

        if let Some(offset) = self.hhdm_offset {
            if virtual_address >= offset {
                return virtual_address - offset;
            }
        }

        virtual_address
    }

    pub fn virtual_for_physical(self, physical_address: u64) -> u64 {
        if self.kernel_length != 0
            && physical_address >= self.kernel_physical_start
            && physical_address
                < self
                    .kernel_physical_start
                    .saturating_add(self.kernel_length)
        {
            return self
                .kernel_virtual_start
                .saturating_add(physical_address.saturating_sub(self.kernel_physical_start));
        }

        if let Some(offset) = self.hhdm_offset {
            return physical_address.saturating_add(offset);
        }

        physical_address
    }
}

#[derive(Clone, Copy)]
struct MappingFlags(u64);

impl MappingFlags {
    const READ_ONLY: Self = Self(PRESENT);
    const WRITABLE: Self = Self(PRESENT | WRITABLE);
    const WRITABLE_NO_EXECUTE: Self = Self(PRESENT | WRITABLE | NO_EXECUTE);
}

/// Construct early page tables and load CR3 once they contain identity and kernel mappings.
pub fn initialize(boot_info: &BootInfo) {
    unsafe {
        let pool = core::ptr::addr_of_mut!(TABLES);
        (*pool).reset();
        let pml4 = (*pool).pml4();

        let translator = AddressTranslator::new(boot_info);

        map_early_identity_ranges(pool, pml4, &translator, boot_info);
        map_kernel_image(pool, pml4, &translator, boot_info);
        map_framebuffer(pool, pml4, &translator, boot_info);

        ACTIVE_TRANSLATOR = translator;
        ACTIVE_PML4 = pml4;
        load_cr3(translator.physical_for_virtual(pml4 as u64));
        INSTALLED.store(true, Ordering::SeqCst);
    }
}

unsafe fn map_early_identity_ranges(
    pool: *mut PageTablePool,
    pml4: *mut PageTable,
    translator: &AddressTranslator,
    boot_info: &BootInfo,
) {
    let mut mapped_any = false;

    if let Some(map) = boot_info.memory_map {
        for index in 0..map.len() {
            if let Some(entry) = map.entry(index) {
                if !is_early_identity_region(entry.kind) {
                    continue;
                }

                let start = align_down(entry.base.0);
                let end = align_up(entry.base.0.saturating_add(entry.length));
                let capped_end = end.min(DEFAULT_IDENTITY_LIMIT);
                if start < capped_end {
                    map_range(
                        pool,
                        pml4,
                        translator,
                        start,
                        start,
                        capped_end.saturating_sub(start),
                        MappingFlags::WRITABLE_NO_EXECUTE,
                    );
                    mapped_any = true;
                }
            }
        }
    }

    if !mapped_any {
        map_range(
            pool,
            pml4,
            translator,
            0,
            0,
            DEFAULT_IDENTITY_LIMIT,
            MappingFlags::WRITABLE_NO_EXECUTE,
        );
    }
}

fn is_early_identity_region(kind: MemoryRegionKind) -> bool {
    matches!(
        kind,
        MemoryRegionKind::Usable
            | MemoryRegionKind::BootloaderReclaimable
            | MemoryRegionKind::KernelAndModules
            | MemoryRegionKind::Framebuffer
            | MemoryRegionKind::AcpiReclaimable
    )
}

unsafe fn map_kernel_image(
    pool: *mut PageTablePool,
    pml4: *mut PageTable,
    translator: &AddressTranslator,
    boot_info: &BootInfo,
) {
    if let Some(load) = boot_info.kernel.load_range {
        map_range(
            pool,
            pml4,
            translator,
            load.physical_start.0,
            load.virtual_start.0,
            load.length,
            MappingFlags::WRITABLE,
        );
    }

    let sections = boot_info.kernel.sections;
    map_virtual_section(
        pool,
        pml4,
        translator,
        sections.text.start.0,
        sections.text.length(),
        MappingFlags::READ_ONLY,
    );
    map_virtual_section(
        pool,
        pml4,
        translator,
        sections.rodata.start.0,
        sections.rodata.length(),
        MappingFlags::READ_ONLY.0 | NO_EXECUTE,
    );
    map_virtual_section(
        pool,
        pml4,
        translator,
        sections.data.start.0,
        sections.data.length(),
        MappingFlags::WRITABLE_NO_EXECUTE,
    );
    map_virtual_section(
        pool,
        pml4,
        translator,
        sections.bss.start.0,
        sections.bss.length(),
        MappingFlags::WRITABLE_NO_EXECUTE,
    );
}

unsafe fn map_virtual_section(
    pool: *mut PageTablePool,
    pml4: *mut PageTable,
    translator: &AddressTranslator,
    virtual_start: u64,
    length: u64,
    flags: impl Into<MappingFlags>,
) {
    if length == 0 {
        return;
    }
    map_range(
        pool,
        pml4,
        translator,
        translator.physical_for_virtual(virtual_start),
        virtual_start,
        length,
        flags.into(),
    );
}

unsafe fn map_framebuffer(
    pool: *mut PageTablePool,
    pml4: *mut PageTable,
    translator: &AddressTranslator,
    boot_info: &BootInfo,
) {
    if let Some(framebuffer) = boot_info.framebuffer {
        let length = framebuffer.pitch.saturating_mul(framebuffer.height);
        map_range(
            pool,
            pml4,
            translator,
            translator.physical_for_virtual(framebuffer.address.0),
            framebuffer.address.0,
            length,
            MappingFlags::WRITABLE_NO_EXECUTE,
        );
    }
}

impl From<u64> for MappingFlags {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

unsafe fn map_range(
    pool: *mut PageTablePool,
    pml4: *mut PageTable,
    translator: &AddressTranslator,
    physical_start: u64,
    virtual_start: u64,
    length: u64,
    flags: MappingFlags,
) {
    let mut physical = align_down(physical_start);
    let mut virtual_address = align_down(virtual_start);
    let end = align_up(virtual_start.saturating_add(length));

    while virtual_address < end {
        map_page(pool, pml4, translator, physical, virtual_address, flags);
        physical = physical.saturating_add(PAGE_SIZE);
        virtual_address = virtual_address.saturating_add(PAGE_SIZE);
    }
}

unsafe fn map_page(
    pool: *mut PageTablePool,
    pml4: *mut PageTable,
    translator: &AddressTranslator,
    physical: u64,
    virtual_address: u64,
    flags: MappingFlags,
) {
    let pml4_index = index(virtual_address, 39);
    let pdpt = next_table(pool, translator, pml4, pml4_index);
    let pdpt_index = index(virtual_address, 30);
    let pd = next_table(pool, translator, pdpt, pdpt_index);
    let pd_index = index(virtual_address, 21);
    let pt = next_table(pool, translator, pd, pd_index);
    let pt_index = index(virtual_address, 12);

    (*pt).entries[pt_index] = (physical & ADDRESS_MASK) | flags.0;
}

unsafe fn next_table(
    pool: *mut PageTablePool,
    translator: &AddressTranslator,
    parent: *mut PageTable,
    index: usize,
) -> *mut PageTable {
    let entry = &mut (*parent).entries[index];
    if *entry & PRESENT == 0 {
        let table = (*pool)
            .allocate()
            .expect("early x86_64 page table pool exhausted");
        *entry =
            (translator.physical_for_virtual(table as u64) & ADDRESS_MASK) | PRESENT | WRITABLE;
    }
    translator.virtual_for_physical(PageTablePool::physical_from_entry(*entry)) as *mut PageTable
}

const fn index(address: u64, shift: u8) -> usize {
    ((address >> shift) & 0x1ff) as usize
}

const fn align_down(address: u64) -> u64 {
    address & !(PAGE_SIZE - 1)
}

const fn align_up(address: u64) -> u64 {
    (address + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

unsafe fn load_cr3(pml4_physical: u64) {
    #[cfg(not(test))]
    core::arch::asm!("mov cr3, {}", in(reg) pml4_physical, options(nostack, preserves_flags));

    #[cfg(test)]
    let _ = pml4_physical;
}

pub fn map_kernel_page(
    physical: u64,
    virtual_address: u64,
    protection: MemoryProtection,
) -> Option<()> {
    unsafe {
        if ACTIVE_PML4.is_null() {
            return None;
        }
        let flags = flags_from_protection(protection);
        let translator = ACTIVE_TRANSLATOR;
        map_page(
            core::ptr::addr_of_mut!(TABLES),
            ACTIVE_PML4,
            &translator,
            physical,
            virtual_address,
            flags,
        );
        Some(())
    }
}

pub fn page_table_pool_range() -> (usize, usize) {
    let start = core::ptr::addr_of!(TABLES) as usize;
    (start, core::mem::size_of::<PageTablePool>())
}

fn flags_from_protection(protection: MemoryProtection) -> MappingFlags {
    let mut flags = PRESENT;
    if protection.write {
        flags |= WRITABLE;
    }
    if !protection.execute {
        flags |= NO_EXECUTE;
    }
    MappingFlags(flags)
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}
