//! Early x86_64 page table construction from bootloader memory information.

use core::sync::atomic::{AtomicBool, Ordering};

use super::boot::{BootInfo, MemoryRegionKind};
use crate::kernel::memory::MemoryProtection;

const PAGE_SIZE: u64 = 4096;
const ENTRY_COUNT: usize = 512;
const MAX_TABLES: usize = 128;
const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const USER_ACCESSIBLE: u64 = 1 << 2;
const WRITE_THROUGH: u64 = 1 << 3;
const CACHE_DISABLE: u64 = 1 << 4;
const GLOBAL: u64 = 1 << 8;
const NO_EXECUTE: u64 = 1 << 63;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const SUPPORTED_PAGE_FLAGS: u64 =
    PRESENT | WRITABLE | USER_ACCESSIBLE | NO_EXECUTE | GLOBAL | WRITE_THROUGH | CACHE_DISABLE;
const DEFAULT_IDENTITY_LIMIT: u64 = 16 * 1024 * 1024;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static mut TABLES: PageTablePool = PageTablePool::new();
static mut ACTIVE_TRANSLATOR: AddressTranslator = AddressTranslator::identity();
static mut ACTIVE_PML4: *mut PageTable = core::ptr::null_mut();
static mut KERNEL_PML4_PHYSICAL: u64 = 0;
static mut CURRENT_ADDRESS_SPACE_ROOT: u64 = 0;
static FRAME_ALLOCATOR_READY: AtomicBool = AtomicBool::new(false);

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PagingError {
    NotInitialized,
    MissingHhdm,
    InvalidAlignment,
    AddressOverflow,
    OutOfPageTables,
    NotMapped,
    UnsupportedFlags,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageFlags(u64);

impl PageFlags {
    pub const PRESENT: Self = Self(PRESENT);
    pub const WRITABLE: Self = Self(WRITABLE);
    pub const USER: Self = Self(USER_ACCESSIBLE);
    pub const NO_EXECUTE: Self = Self(NO_EXECUTE);
    pub const GLOBAL: Self = Self(GLOBAL);
    pub const WRITE_THROUGH: Self = Self(WRITE_THROUGH);
    pub const CACHE_DISABLE: Self = Self(CACHE_DISABLE);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn from_bits(bits: u64) -> Option<Self> {
        if bits & !SUPPORTED_PAGE_FLAGS == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    const fn kernel_table() -> Self {
        Self(PRESENT | WRITABLE)
    }

    const fn ensure_present(self) -> Self {
        Self(self.0 | PRESENT)
    }
}

impl core::ops::BitOr for PageFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for PageFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
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

    const fn into_page_flags(self) -> PageFlags {
        PageFlags(self.0)
    }
}

/// Capture the bootloader-installed kernel address space and keep the static
/// table builder only as a pre-allocator fallback for environments without an
/// HHDM mapping.
pub fn initialize(boot_info: &BootInfo) {
    unsafe {
        let translator = AddressTranslator::new(boot_info);
        ACTIVE_TRANSLATOR = translator;

        if boot_info.hhdm_offset.is_some() {
            let root = read_cr3();
            KERNEL_PML4_PHYSICAL = root;
            CURRENT_ADDRESS_SPACE_ROOT = root;
            ACTIVE_PML4 = translator.virtual_for_physical(root) as *mut PageTable;
            INSTALLED.store(true, Ordering::SeqCst);
            return;
        }

        let pool = core::ptr::addr_of_mut!(TABLES);
        (*pool).reset();
        let pml4 = (*pool).pml4();

        let _ = map_early_identity_ranges(pool, pml4, &translator, boot_info);
        let _ = map_kernel_image(pool, pml4, &translator, boot_info);
        let _ = map_framebuffer(pool, pml4, &translator, boot_info);

        ACTIVE_PML4 = pml4;
        KERNEL_PML4_PHYSICAL = translator.physical_for_virtual(pml4 as u64);
        CURRENT_ADDRESS_SPACE_ROOT = KERNEL_PML4_PHYSICAL;
        load_cr3(KERNEL_PML4_PHYSICAL);
        INSTALLED.store(true, Ordering::SeqCst);
    }
}

/// Enable frame-backed page-table growth after the physical allocator has
/// ingested the boot memory map.
pub fn enable_frame_backed_mapping(boot_info: &BootInfo) -> Result<(), PagingError> {
    if boot_info.hhdm_offset.is_none() {
        return Err(PagingError::MissingHhdm);
    }
    unsafe {
        ACTIVE_TRANSLATOR = AddressTranslator::new(boot_info);
        if KERNEL_PML4_PHYSICAL == 0 {
            KERNEL_PML4_PHYSICAL = read_cr3();
            CURRENT_ADDRESS_SPACE_ROOT = KERNEL_PML4_PHYSICAL;
        }
        ACTIVE_PML4 =
            ACTIVE_TRANSLATOR.virtual_for_physical(KERNEL_PML4_PHYSICAL) as *mut PageTable;
    }
    FRAME_ALLOCATOR_READY.store(true, Ordering::SeqCst);
    Ok(())
}

unsafe fn map_early_identity_ranges(
    pool: *mut PageTablePool,
    pml4: *mut PageTable,
    translator: &AddressTranslator,
    boot_info: &BootInfo,
) -> Result<(), PagingError> {
    let mut mapped_any = false;

    if let Some(map) = boot_info.memory_map {
        for index in 0..map.len() {
            if let Some(entry) = map.entry(index) {
                if !is_early_identity_region(entry.kind) {
                    continue;
                }

                let start = align_down(entry.base.0);
                let end = align_up_checked(
                    entry
                        .base
                        .0
                        .checked_add(entry.length)
                        .ok_or(PagingError::AddressOverflow)?,
                )?;
                let capped_end = end.min(DEFAULT_IDENTITY_LIMIT);
                if start < capped_end {
                    map_range_inner(
                        pool,
                        pml4,
                        translator,
                        start,
                        start,
                        capped_end.saturating_sub(start),
                        MappingFlags::WRITABLE_NO_EXECUTE,
                    )?;
                    mapped_any = true;
                }
            }
        }
    }

    if !mapped_any {
        map_range_inner(
            pool,
            pml4,
            translator,
            0,
            0,
            DEFAULT_IDENTITY_LIMIT,
            MappingFlags::WRITABLE_NO_EXECUTE,
        )?;
    }
    Ok(())
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
) -> Result<(), PagingError> {
    if let Some(load) = boot_info.kernel.load_range {
        map_range_inner(
            pool,
            pml4,
            translator,
            load.physical_start.0,
            load.virtual_start.0,
            load.length,
            MappingFlags::WRITABLE,
        )?;
    }

    let sections = boot_info.kernel.sections;
    map_virtual_section(
        pool,
        pml4,
        translator,
        sections.text.start.0,
        sections.text.length(),
        MappingFlags::READ_ONLY,
    )?;
    map_virtual_section(
        pool,
        pml4,
        translator,
        sections.rodata.start.0,
        sections.rodata.length(),
        MappingFlags::READ_ONLY.0 | NO_EXECUTE,
    )?;
    map_virtual_section(
        pool,
        pml4,
        translator,
        sections.data.start.0,
        sections.data.length(),
        MappingFlags::WRITABLE_NO_EXECUTE,
    )?;
    map_virtual_section(
        pool,
        pml4,
        translator,
        sections.bss.start.0,
        sections.bss.length(),
        MappingFlags::WRITABLE_NO_EXECUTE,
    )
}

unsafe fn map_virtual_section(
    pool: *mut PageTablePool,
    pml4: *mut PageTable,
    translator: &AddressTranslator,
    virtual_start: u64,
    length: u64,
    flags: impl Into<MappingFlags>,
) -> Result<(), PagingError> {
    if length == 0 {
        return Ok(());
    }
    map_range_inner(
        pool,
        pml4,
        translator,
        translator.physical_for_virtual(virtual_start),
        virtual_start,
        length,
        flags.into(),
    )
}

unsafe fn map_framebuffer(
    pool: *mut PageTablePool,
    pml4: *mut PageTable,
    translator: &AddressTranslator,
    boot_info: &BootInfo,
) -> Result<(), PagingError> {
    if let Some(framebuffer) = boot_info.framebuffer {
        let length = framebuffer
            .pitch
            .checked_mul(framebuffer.height)
            .ok_or(PagingError::AddressOverflow)?;
        // TODO(framebuffer): select PAT/write-combining attributes once Mirage
        // has a scoped PAT manager. The early mapping remains cache-default.
        map_range_inner(
            pool,
            pml4,
            translator,
            translator.physical_for_virtual(framebuffer.address.0),
            framebuffer.address.0,
            length,
            MappingFlags::WRITABLE_NO_EXECUTE,
        )?;
    }
    Ok(())
}

impl From<u64> for MappingFlags {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

unsafe fn map_range_inner(
    pool: *mut PageTablePool,
    pml4: *mut PageTable,
    translator: &AddressTranslator,
    physical_start: u64,
    virtual_start: u64,
    length: u64,
    flags: MappingFlags,
) -> Result<(), PagingError> {
    let mut physical = align_down(physical_start);
    let mut virtual_address = align_down(virtual_start);
    let end = align_up_checked(
        virtual_start
            .checked_add(length)
            .ok_or(PagingError::AddressOverflow)?,
    )?;

    while virtual_address < end {
        map_page_inner(pool, pml4, translator, physical, virtual_address, flags)?;
        physical = physical
            .checked_add(PAGE_SIZE)
            .ok_or(PagingError::AddressOverflow)?;
        virtual_address = virtual_address
            .checked_add(PAGE_SIZE)
            .ok_or(PagingError::AddressOverflow)?;
    }
    Ok(())
}

unsafe fn map_page_inner(
    pool: *mut PageTablePool,
    pml4: *mut PageTable,
    translator: &AddressTranslator,
    physical: u64,
    virtual_address: u64,
    flags: MappingFlags,
) -> Result<(), PagingError> {
    let pml4_index = index(virtual_address, 39);
    let pdpt = next_table(pool, translator, pml4, pml4_index)?;
    let pdpt_index = index(virtual_address, 30);
    let pd = next_table(pool, translator, pdpt, pdpt_index)?;
    let pd_index = index(virtual_address, 21);
    let pt = next_table(pool, translator, pd, pd_index)?;
    let pt_index = index(virtual_address, 12);

    let old = (*pt).entries[pt_index];
    (*pt).entries[pt_index] = (physical & ADDRESS_MASK) | flags.0;
    if old & PRESENT != 0 {
        invalidate_page(virtual_address);
    }
    Ok(())
}

unsafe fn next_table(
    pool: *mut PageTablePool,
    translator: &AddressTranslator,
    parent: *mut PageTable,
    index: usize,
) -> Result<*mut PageTable, PagingError> {
    let entry = &mut (*parent).entries[index];
    if *entry & PRESENT == 0 {
        let (table, physical) = allocate_page_table(pool, translator)?;
        (*table).entries.fill(0);
        *entry = (physical & ADDRESS_MASK) | PageFlags::kernel_table().bits();
    }
    Ok(
        translator.virtual_for_physical(PageTablePool::physical_from_entry(*entry))
            as *mut PageTable,
    )
}

unsafe fn allocate_page_table(
    pool: *mut PageTablePool,
    translator: &AddressTranslator,
) -> Result<(*mut PageTable, u64), PagingError> {
    if FRAME_ALLOCATOR_READY.load(Ordering::SeqCst) {
        let physical =
            crate::kernel::memory::allocate_physical_frame().ok_or(PagingError::OutOfPageTables)?;
        let table = translator.virtual_for_physical(physical) as *mut PageTable;
        return Ok((table, physical));
    }

    let table = (*pool).allocate().ok_or(PagingError::OutOfPageTables)?;
    Ok((table, translator.physical_for_virtual(table as u64)))
}

const fn index(address: u64, shift: u8) -> usize {
    ((address >> shift) & 0x1ff) as usize
}

const fn align_down(address: u64) -> u64 {
    address & !(PAGE_SIZE - 1)
}

fn align_up_checked(address: u64) -> Result<u64, PagingError> {
    address
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value & !(PAGE_SIZE - 1))
        .ok_or(PagingError::AddressOverflow)
}

fn validate_page_alignment(virtual_address: u64, physical: u64) -> Result<(), PagingError> {
    if virtual_address & (PAGE_SIZE - 1) != 0 || physical & (PAGE_SIZE - 1) != 0 {
        return Err(PagingError::InvalidAlignment);
    }
    Ok(())
}

unsafe fn load_cr3(pml4_physical: u64) {
    #[cfg(not(test))]
    core::arch::asm!("mov cr3, {}", in(reg) pml4_physical, options(nostack, preserves_flags));

    #[cfg(test)]
    let _ = pml4_physical;
}

fn read_cr3() -> u64 {
    #[cfg(all(not(test), target_arch = "x86_64"))]
    unsafe {
        let value: u64;
        core::arch::asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
        value
    }

    #[cfg(any(test, not(target_arch = "x86_64")))]
    {
        0
    }
}

fn invalidate_page(virtual_address: u64) {
    #[cfg(all(not(test), target_arch = "x86_64"))]
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) virtual_address, options(nostack, preserves_flags));
    }

    #[cfg(any(test, not(target_arch = "x86_64")))]
    let _ = virtual_address;
}

pub fn map_kernel_page(
    physical: u64,
    virtual_address: u64,
    protection: MemoryProtection,
) -> Option<()> {
    map_page(
        virtual_address,
        physical,
        flags_from_protection(protection).into_page_flags(),
    )
    .ok()
}

pub fn map_page(virt: u64, phys: u64, flags: PageFlags) -> Result<(), PagingError> {
    validate_page_alignment(virt, phys)?;
    if flags.bits() & !SUPPORTED_PAGE_FLAGS != 0 {
        return Err(PagingError::UnsupportedFlags);
    }
    unsafe {
        if ACTIVE_PML4.is_null() {
            return Err(PagingError::NotInitialized);
        }
        let translator = ACTIVE_TRANSLATOR;
        map_page_inner(
            core::ptr::addr_of_mut!(TABLES),
            ACTIVE_PML4,
            &translator,
            phys,
            virt,
            MappingFlags(flags.ensure_present().bits()),
        )
    }
}

pub fn unmap_page(virt: u64) -> Result<u64, PagingError> {
    validate_page_alignment(virt, 0)?;
    unsafe {
        if ACTIVE_PML4.is_null() {
            return Err(PagingError::NotInitialized);
        }
        let pt = leaf_table(ACTIVE_PML4, virt).ok_or(PagingError::NotMapped)?;
        let slot = index(virt, 12);
        let entry = (*pt).entries[slot];
        if entry & PRESENT == 0 {
            return Err(PagingError::NotMapped);
        }
        (*pt).entries[slot] = 0;
        invalidate_page(virt);
        Ok(entry & ADDRESS_MASK)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageTableWalk {
    pub cr3: u64,
    pub pml4e: u64,
    pub pdpte: u64,
    pub pde: u64,
    pub pte: u64,
    pub physical: Option<u64>,
}

pub fn walk_kernel_page_tables(virtual_address: u64) -> Option<PageTableWalk> {
    unsafe {
        if ACTIVE_PML4.is_null() {
            return None;
        }
        let pml4e = (*ACTIVE_PML4).entries[index(virtual_address, 39)];
        if pml4e & PRESENT == 0 {
            return Some(PageTableWalk {
                cr3: CURRENT_ADDRESS_SPACE_ROOT,
                pml4e,
                pdpte: 0,
                pde: 0,
                pte: 0,
                physical: None,
            });
        }
        let pdpt = table_for_physical(pml4e & ADDRESS_MASK);
        let pdpte = (*pdpt).entries[index(virtual_address, 30)];
        if pdpte & PRESENT == 0 {
            return Some(PageTableWalk {
                cr3: CURRENT_ADDRESS_SPACE_ROOT,
                pml4e,
                pdpte,
                pde: 0,
                pte: 0,
                physical: None,
            });
        }
        let pd = table_for_physical(pdpte & ADDRESS_MASK);
        let pde = (*pd).entries[index(virtual_address, 21)];
        if pde & PRESENT == 0 {
            return Some(PageTableWalk {
                cr3: CURRENT_ADDRESS_SPACE_ROOT,
                pml4e,
                pdpte,
                pde,
                pte: 0,
                physical: None,
            });
        }
        let pt = table_for_physical(pde & ADDRESS_MASK);
        let pte = (*pt).entries[index(virtual_address, 12)];
        let physical = if pte & PRESENT != 0 {
            Some((pte & ADDRESS_MASK) | (virtual_address & (PAGE_SIZE - 1)))
        } else {
            None
        };
        Some(PageTableWalk {
            cr3: CURRENT_ADDRESS_SPACE_ROOT,
            pml4e,
            pdpte,
            pde,
            pte,
            physical,
        })
    }
}

pub fn translate_virt(virt: u64) -> Option<u64> {
    unsafe {
        if ACTIVE_PML4.is_null() {
            return None;
        }
        let pt = leaf_table(ACTIVE_PML4, virt)?;
        let pte = (*pt).entries[index(virt, 12)];
        if pte & PRESENT == 0 {
            return None;
        }
        Some((pte & ADDRESS_MASK) | (virt & (PAGE_SIZE - 1)))
    }
}

pub fn map_range(
    virt_start: u64,
    phys_start: u64,
    length: u64,
    flags: PageFlags,
) -> Result<(), PagingError> {
    validate_page_alignment(virt_start, phys_start)?;
    if length == 0 || length & (PAGE_SIZE - 1) != 0 {
        return Err(PagingError::InvalidAlignment);
    }
    virt_start
        .checked_add(length)
        .ok_or(PagingError::AddressOverflow)?;
    phys_start
        .checked_add(length)
        .ok_or(PagingError::AddressOverflow)?;

    let mut offset = 0;
    while offset < length {
        map_page(virt_start + offset, phys_start + offset, flags)?;
        offset = offset
            .checked_add(PAGE_SIZE)
            .ok_or(PagingError::AddressOverflow)?;
    }
    Ok(())
}

pub fn hhdm_virt_for_phys(physical: u64) -> Option<u64> {
    unsafe {
        ACTIVE_TRANSLATOR
            .hhdm_offset
            .and_then(|offset| physical.checked_add(offset))
    }
}

pub fn hhdm_phys_for_virt(virtual_address: u64) -> Option<u64> {
    unsafe {
        let offset = ACTIVE_TRANSLATOR.hhdm_offset?;
        virtual_address.checked_sub(offset)
    }
}

unsafe fn leaf_table(pml4: *mut PageTable, virtual_address: u64) -> Option<*mut PageTable> {
    let pml4e = (*pml4).entries[index(virtual_address, 39)];
    if pml4e & PRESENT == 0 {
        return None;
    }
    let pdpt = table_for_physical(pml4e & ADDRESS_MASK);
    let pdpte = (*pdpt).entries[index(virtual_address, 30)];
    if pdpte & PRESENT == 0 {
        return None;
    }
    let pd = table_for_physical(pdpte & ADDRESS_MASK);
    let pde = (*pd).entries[index(virtual_address, 21)];
    if pde & PRESENT == 0 {
        return None;
    }
    Some(table_for_physical(pde & ADDRESS_MASK))
}

fn table_for_physical(physical: u64) -> *mut PageTable {
    unsafe { ACTIVE_TRANSLATOR.virtual_for_physical(physical) as *mut PageTable }
}

pub fn active_translator() -> AddressTranslator {
    unsafe { ACTIVE_TRANSLATOR }
}

pub fn kernel_address_space_root() -> u64 {
    unsafe { KERNEL_PML4_PHYSICAL }
}

pub fn current_address_space_root() -> u64 {
    unsafe { CURRENT_ADDRESS_SPACE_ROOT }
}

pub fn switch_address_space(root: u64) -> Option<()> {
    unsafe {
        let target = if root == 0 {
            KERNEL_PML4_PHYSICAL
        } else {
            root
        };
        if target == 0 {
            return None;
        }
        if CURRENT_ADDRESS_SPACE_ROOT != target {
            load_cr3(target);
            CURRENT_ADDRESS_SPACE_ROOT = target;
        }
        Some(())
    }
}

pub fn create_user_address_space() -> Option<u64> {
    let frame = crate::kernel::memory::allocate_physical_frame()?;
    unsafe {
        let pml4 = table_for_physical(frame);
        (*pml4).entries.fill(0);
        if !ACTIVE_PML4.is_null() {
            let mut idx = 256usize;
            while idx < ENTRY_COUNT {
                (*pml4).entries[idx] = (*ACTIVE_PML4).entries[idx];
                idx += 1;
            }
        }
    }
    Some(frame)
}

pub fn destroy_user_address_space(root: u64) {
    if root == 0 || root == unsafe { KERNEL_PML4_PHYSICAL } {
        return;
    }
    unsafe {
        let pml4 = table_for_physical(root);
        let mut pml4_idx = 0usize;
        while pml4_idx < 256 {
            let pml4e = (*pml4).entries[pml4_idx];
            if pml4e & PRESENT != 0 {
                let pdpt_phys = pml4e & ADDRESS_MASK;
                let pdpt = table_for_physical(pdpt_phys);
                let mut pdpt_idx = 0usize;
                while pdpt_idx < ENTRY_COUNT {
                    let pdpte = (*pdpt).entries[pdpt_idx];
                    if pdpte & PRESENT != 0 {
                        let pd_phys = pdpte & ADDRESS_MASK;
                        let pd = table_for_physical(pd_phys);
                        let mut pd_idx = 0usize;
                        while pd_idx < ENTRY_COUNT {
                            let pde = (*pd).entries[pd_idx];
                            if pde & PRESENT != 0 {
                                let pt_phys = pde & ADDRESS_MASK;
                                crate::kernel::memory::deallocate_physical_frame(pt_phys);
                            }
                            pd_idx += 1;
                        }
                        crate::kernel::memory::deallocate_physical_frame(pd_phys);
                    }
                    pdpt_idx += 1;
                }
                crate::kernel::memory::deallocate_physical_frame(pdpt_phys);
            }
            pml4_idx += 1;
        }
        crate::kernel::memory::deallocate_physical_frame(root);
    }
}

unsafe fn user_next_table(parent: *mut PageTable, slot: usize) -> Option<*mut PageTable> {
    let entry = &mut (*parent).entries[slot];
    if *entry & PRESENT == 0 {
        let frame = crate::kernel::memory::allocate_physical_frame()?;
        let table = table_for_physical(frame);
        (*table).entries.fill(0);
        *entry = (frame & ADDRESS_MASK) | PRESENT | WRITABLE | USER_ACCESSIBLE;
    }
    Some(table_for_physical(*entry & ADDRESS_MASK))
}

pub fn translate_kernel_address(virtual_address: u64) -> Option<u64> {
    translate_virt(virtual_address)
}

pub fn map_user_page(
    root: u64,
    virtual_address: u64,
    physical: u64,
    protection: MemoryProtection,
) -> Option<()> {
    if root == 0
        || virtual_address >= 0x0000_8000_0000_0000
        || virtual_address & (PAGE_SIZE - 1) != 0
    {
        return None;
    }
    unsafe {
        let pml4 = table_for_physical(root);
        let pdpt = user_next_table(pml4, index(virtual_address, 39))?;
        let pd = user_next_table(pdpt, index(virtual_address, 30))?;
        let pt = user_next_table(pd, index(virtual_address, 21))?;
        let flags = user_flags_from_protection(protection);
        (*pt).entries[index(virtual_address, 12)] = (physical & ADDRESS_MASK) | flags.0;
        Some(())
    }
}

pub fn unmap_user_page(root: u64, virtual_address: u64) -> Option<u64> {
    let pt = user_leaf_table(root, virtual_address)?;
    unsafe {
        let slot = index(virtual_address, 12);
        let entry = (*pt).entries[slot];
        if entry & PRESENT == 0 {
            return None;
        }
        (*pt).entries[slot] = 0;
        Some(entry & ADDRESS_MASK)
    }
}

pub fn translate_user_page(root: u64, virtual_address: u64, write: bool) -> Option<u64> {
    let pt = user_leaf_table(root, virtual_address)?;
    unsafe {
        let entry = (*pt).entries[index(virtual_address, 12)];
        if entry & PRESENT == 0 || entry & USER_ACCESSIBLE == 0 {
            return None;
        }
        if write && entry & WRITABLE == 0 {
            return None;
        }
        Some((entry & ADDRESS_MASK) | (virtual_address & (PAGE_SIZE - 1)))
    }
}

fn user_leaf_table(root: u64, virtual_address: u64) -> Option<*mut PageTable> {
    if root == 0 || virtual_address >= 0x0000_8000_0000_0000 {
        return None;
    }
    unsafe {
        let pml4 = table_for_physical(root);
        let pml4e = (*pml4).entries[index(virtual_address, 39)];
        if pml4e & PRESENT == 0 {
            return None;
        }
        let pdpt = table_for_physical(pml4e & ADDRESS_MASK);
        let pdpte = (*pdpt).entries[index(virtual_address, 30)];
        if pdpte & PRESENT == 0 {
            return None;
        }
        let pd = table_for_physical(pdpte & ADDRESS_MASK);
        let pde = (*pd).entries[index(virtual_address, 21)];
        if pde & PRESENT == 0 {
            return None;
        }
        Some(table_for_physical(pde & ADDRESS_MASK))
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

fn user_flags_from_protection(protection: MemoryProtection) -> MappingFlags {
    let mut flags = PRESENT | USER_ACCESSIBLE;
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

pub fn frame_backed_mapping_enabled() -> bool {
    FRAME_ALLOCATOR_READY.load(Ordering::SeqCst)
}
