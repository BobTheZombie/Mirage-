//! Earliest x86_64 bootstrap and typed boot information handoff.
//!
//! Limine enters the kernel in 64-bit mode with paging enabled. The assembly entry below still
//! takes ownership of the initial stack contract, aligns it to the SysV x86_64 ABI, and
//! then calls the Rust seed-rs handoff layer.

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
use core::arch::global_asm;
#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
use core::ptr::addr_of;

use crate::boot::{self as limine, Framebuffer};
use crate::boot::{LimineFile, MemoryMapEntry as LimineMemoryMapEntry};

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
global_asm!(
    r#"
    .section .text._start,"ax",@progbits
    .global _start
    .type _start,@function
_start:
    lea rsp, [rip + __stack_top]
    and rsp, -16
    xor rbp, rbp
    call __mirage_x86_64_seed_entry
.Lmirage_boot_hang:
    hlt
    jmp .Lmirage_boot_hang
    .size _start, . - _start
"#
);

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
extern "C" {
    static mut __bss_start: u8;
    static mut __bss_end: u8;
    static __kernel_start: u8;
    static __kernel_end: u8;
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
}

/// Architecture-owned entry point called by the `_start` assembly stub.
#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
#[no_mangle]
pub unsafe extern "C" fn __mirage_x86_64_seed_entry() -> ! {
    crate::arch::x86_64::seed_rs::x86_64_handoff()
}

/// Backward-compatible symbol for older diagnostics; the seed entry is now the
/// only handoff path used by `_start`.
#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
#[no_mangle]
pub unsafe extern "C" fn __mirage_x86_64_bootstrap() -> ! {
    __mirage_x86_64_seed_entry()
}

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
pub unsafe fn clear_bss() {
    let start = addr_of!(__bss_start) as usize;
    let end = addr_of!(__bss_end) as usize;
    let mut cursor = start;

    while cursor < end {
        core::ptr::write_volatile(cursor as *mut u8, 0);
        cursor = cursor.saturating_add(1);
    }
}

#[cfg(all(
    not(test),
    not(feature = "qfs-std"),
    target_os = "none",
    feature = "boot-trace"
))]
fn bootinfo_marker(message: &str) {
    unsafe {
        crate::arch::x86_64::seed_rs::seed_com1_write_str(message);
    }
}

#[cfg(any(
    test,
    feature = "qfs-std",
    not(target_os = "none"),
    not(feature = "boot-trace")
))]
fn bootinfo_marker(_message: &str) {}

const BOOT_CSTRING_SCAN_LIMIT: usize = 256;

fn is_aligned<T>(ptr: *const T) -> bool {
    (ptr as usize) % core::mem::align_of::<T>() == 0
}

/// Parsed boot information passed from the architecture bootstrap to the kernel proper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootInfo {
    pub boot_protocol: BootProtocol,
    pub bootloader: BootloaderInfo,
    pub memory_map: Option<MemoryMap>,
    pub framebuffer: Option<FramebufferInfo>,
    pub serial: Option<SerialInfo>,
    pub rsdp: Option<PhysicalAddress>,
    pub hhdm_offset: Option<u64>,
    pub kernel: KernelImageInfo,
    pub modules: BootModules,
}

impl BootInfo {
    /// Return whether the active boot protocol satisfied Mirage's required base revision.
    ///
    /// This must be checked before normal architecture initialization so the
    /// kernel does not continue after an incompatible Limine handoff.
    pub const fn limine_base_revision_supported(self) -> bool {
        match self.boot_protocol {
            BootProtocol::Limine {
                base_revision_supported,
            } => base_revision_supported,
        }
    }

    #[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
    pub(crate) fn from_limine(raw: limine::LimineBootSnapshot, sections: KernelSections) -> Self {
        bootinfo_marker("[bootinfo 01] enter from_limine\r\n");

        let executable = raw.executable_address.map(|address| KernelLoadRange {
            physical_start: PhysicalAddress(address.physical_base),
            virtual_start: VirtualAddress(address.virtual_base),
            length: sections.kernel.length(),
        });
        bootinfo_marker("[bootinfo 02] executable address parsed\r\n");

        let boot_protocol = BootProtocol::Limine {
            base_revision_supported: raw.base_revision_supported,
        };
        bootinfo_marker("[bootinfo 03] boot protocol parsed\r\n");

        let bootloader = raw
            .bootloader
            .map(|info| BootloaderInfo {
                name: BootString::from_cstr(info.name),
                version: BootString::from_cstr(info.version),
            })
            .unwrap_or(BootloaderInfo::unknown());
        bootinfo_marker("[bootinfo 04] bootloader parsed\r\n");

        let memory_map = MemoryMap::from_limine_response(raw.memory_map);
        bootinfo_marker("[bootinfo 05] memory map parsed\r\n");

        let framebuffer = first_framebuffer(raw.framebuffer);
        bootinfo_marker("[bootinfo 06] framebuffer parsed\r\n");

        let serial = None;
        bootinfo_marker("[bootinfo 07] serial parsed\r\n");

        let rsdp = raw.rsdp.map(|rsdp| PhysicalAddress(rsdp.address));
        bootinfo_marker("[bootinfo 08] rsdp parsed\r\n");

        let hhdm_offset = raw.hhdm.map(|hhdm| hhdm.offset);
        bootinfo_marker("[bootinfo 09] hhdm parsed\r\n");

        let kernel = KernelImageInfo {
            sections,
            load_range: executable,
        };
        bootinfo_marker("[bootinfo 10] kernel image parsed\r\n");

        let modules = BootModules::from_limine_response(raw.modules);
        bootinfo_marker("[bootinfo 11] modules parsed\r\n");

        let boot_info = Self {
            boot_protocol,
            bootloader,
            memory_map,
            framebuffer,
            serial,
            rsdp,
            hhdm_offset,
            kernel,
            modules,
        };
        bootinfo_marker("[bootinfo 12] BootInfo return\r\n");
        boot_info
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootProtocol {
    Limine { base_revision_supported: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootloaderInfo {
    pub name: BootString,
    pub version: BootString,
}

impl BootloaderInfo {
    pub const fn unknown() -> Self {
        Self {
            name: BootString::empty(),
            version: BootString::empty(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootString {
    pub ptr: *const u8,
    pub len: usize,
}

impl BootString {
    pub const fn empty() -> Self {
        Self {
            ptr: core::ptr::null(),
            len: 0,
        }
    }

    pub fn from_cstr(ptr: *const u8) -> Self {
        bootinfo_marker("[bootstr] before null check\r\n");
        if ptr.is_null() {
            bootinfo_marker("[bootstr] null pointer return\r\n");
            return Self::empty();
        }

        bootinfo_marker("[bootstr] before scan\r\n");
        let mut len = 0;
        while len < BOOT_CSTRING_SCAN_LIMIT {
            let byte = unsafe { ptr.add(len).read() };
            if byte == 0 {
                break;
            }
            len += 1;
        }
        bootinfo_marker("[bootstr] after scan\r\n");

        Self { ptr, len }
    }

    pub fn as_bytes(self) -> &'static [u8] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootModules {
    entries: *const *const LimineFile,
    entry_count: u64,
}

impl BootModules {
    pub const fn empty() -> Self {
        Self {
            entries: core::ptr::null(),
            entry_count: 0,
        }
    }

    pub const fn len(self) -> u64 {
        self.entry_count
    }

    pub const fn is_empty(self) -> bool {
        self.entry_count == 0
    }

    pub fn from_limine_response(response: Option<&limine::ModuleResponse>) -> Self {
        let Some(response) = response else {
            bootinfo_marker("[bootmods] response missing\r\n");
            return Self::empty();
        };

        let module_count = response.module_count;
        bootinfo_marker("[bootmods] module_count read\r\n");
        if module_count == 0 {
            bootinfo_marker("[bootmods] BootModules return\r\n");
            return Self::empty();
        }

        let modules = response.modules;
        if modules.is_null() {
            bootinfo_marker("[bootmods] modules pointer null\r\n");
            bootinfo_marker("[bootmods] BootModules return\r\n");
            return Self::empty();
        }
        if !is_aligned(modules) {
            bootinfo_marker("[bootmods] modules pointer unaligned\r\n");
            bootinfo_marker("[bootmods] BootModules return\r\n");
            return Self::empty();
        }

        bootinfo_marker("[bootmods] BootModules return\r\n");
        Self {
            entries: modules,
            entry_count: module_count,
        }
    }

    pub fn module(self, index: u64) -> Option<BootModule> {
        if index >= self.entry_count || self.entries.is_null() || !is_aligned(self.entries) {
            return None;
        }
        let offset = usize::try_from(index).ok()?;
        let file_ptr = unsafe { self.entries.add(offset).read() };
        if file_ptr.is_null() || !is_aligned(file_ptr) {
            return None;
        }
        let file = unsafe { file_ptr.as_ref()? };
        Some(BootModule {
            base: VirtualAddress(file.address as u64),
            size: file.size,
            path: BootString::from_cstr(file.path),
            command_line: BootString::from_cstr(file.cmdline),
            trust: BootModuleTrust {
                media_type: BootModuleMediaType::from_limine(file.media_type),
                partition_index: file.partition_index,
                mbr_disk_id: file.mbr_disk_id,
                gpt_disk_uuid: BootUuid::from_limine(file.gpt_disk_uuid),
                gpt_part_uuid: BootUuid::from_limine(file.gpt_part_uuid),
                part_uuid: BootUuid::from_limine(file.part_uuid),
                tftp_ip: file.tftp_ip,
                tftp_port: file.tftp_port,
            },
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootModule {
    pub base: VirtualAddress,
    pub size: u64,
    pub path: BootString,
    pub command_line: BootString,
    pub trust: BootModuleTrust,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootModuleTrust {
    pub media_type: BootModuleMediaType,
    pub partition_index: u32,
    pub mbr_disk_id: u32,
    pub gpt_disk_uuid: BootUuid,
    pub gpt_part_uuid: BootUuid,
    pub part_uuid: BootUuid,
    pub tftp_ip: u32,
    pub tftp_port: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootModuleMediaType {
    Generic,
    Optical,
    Tftp,
    Unknown(u32),
}

impl BootModuleMediaType {
    fn from_limine(value: u32) -> Self {
        match value {
            0 => Self::Generic,
            1 => Self::Optical,
            2 => Self::Tftp,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootUuid {
    pub a: u32,
    pub b: u16,
    pub c: u16,
    pub d: [u8; 8],
}

impl BootUuid {
    fn from_limine(value: crate::boot::LimineUuid) -> Self {
        Self {
            a: value.a,
            b: value.b,
            c: value.c,
            d: value.d,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryMap {
    entries: *const *const LimineMemoryMapEntry,
    entry_count: u64,
}

impl MemoryMap {
    pub const fn empty() -> Self {
        Self {
            entries: core::ptr::null(),
            entry_count: 0,
        }
    }

    pub const fn len(self) -> u64 {
        self.entry_count
    }

    pub const fn is_empty(self) -> bool {
        self.entry_count == 0
    }

    pub fn from_limine_response(response: Option<&limine::MemoryMapResponse>) -> Option<Self> {
        let Some(response) = response else {
            bootinfo_marker("[memmap] response missing\r\n");
            return None;
        };

        let entry_count = response.entry_count;
        bootinfo_marker("[memmap] entry_count read\r\n");
        if entry_count == 0 {
            bootinfo_marker("[memmap] MemoryMap return\r\n");
            return None;
        }

        let entries = response.entries;
        if entries.is_null() {
            bootinfo_marker("[memmap] entries pointer null\r\n");
            return None;
        }
        if !is_aligned(entries) {
            bootinfo_marker("[memmap] entries pointer unaligned\r\n");
            return None;
        }

        bootinfo_marker("[memmap] MemoryMap return\r\n");
        Some(Self {
            entries,
            entry_count,
        })
    }

    pub fn entry(self, index: u64) -> Option<MemoryMapEntry> {
        if index >= self.entry_count || self.entries.is_null() || !is_aligned(self.entries) {
            return None;
        }

        let offset = usize::try_from(index).ok()?;
        let entry_ptr = unsafe { self.entries.add(offset).read() };
        if entry_ptr.is_null() || !is_aligned(entry_ptr) {
            return None;
        }
        let entry = unsafe { entry_ptr.as_ref()? };
        Some(MemoryMapEntry {
            base: PhysicalAddress(entry.base),
            length: entry.length,
            kind: MemoryRegionKind::from_limine(entry.entry_type),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryMapEntry {
    pub base: PhysicalAddress,
    pub length: u64,
    pub kind: MemoryRegionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryRegionKind {
    Usable,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    BadMemory,
    BootloaderReclaimable,
    KernelAndModules,
    Framebuffer,
    Unknown(u64),
}

impl MemoryRegionKind {
    fn from_limine(kind: u64) -> Self {
        match kind {
            0 => Self::Usable,
            1 => Self::Reserved,
            2 => Self::AcpiReclaimable,
            3 => Self::AcpiNvs,
            4 => Self::BadMemory,
            5 => Self::BootloaderReclaimable,
            6 => Self::KernelAndModules,
            7 => Self::Framebuffer,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramebufferInfo {
    pub address: VirtualAddress,
    pub width: u64,
    pub height: u64,
    pub pitch: u64,
    pub bits_per_pixel: u16,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
}

fn first_framebuffer(response: Option<&limine::FramebufferResponse>) -> Option<FramebufferInfo> {
    bootinfo_marker("[fb] enter\r\n");
    let Some(response) = response else {
        bootinfo_marker("[fb] response missing\r\n");
        return None;
    };

    let framebuffer_count = response.framebuffer_count;
    bootinfo_marker("[fb] framebuffer_count read\r\n");
    if framebuffer_count == 0 {
        return None;
    }

    let framebuffers = response.framebuffers;
    if framebuffers.is_null() {
        bootinfo_marker("[fb] framebuffers pointer null\r\n");
        return None;
    }
    if !is_aligned(framebuffers) {
        bootinfo_marker("[fb] framebuffers pointer unaligned\r\n");
        return None;
    }

    let framebuffer_ptr = unsafe { framebuffers.read() };
    bootinfo_marker("[fb] first framebuffer pointer read\r\n");
    if framebuffer_ptr.is_null() || !is_aligned(framebuffer_ptr) {
        return None;
    }

    let framebuffer: &Framebuffer = unsafe { framebuffer_ptr.as_ref()? };
    bootinfo_marker("[fb] framebuffer ref acquired\r\n");
    let info = FramebufferInfo {
        address: VirtualAddress(framebuffer.address as u64),
        width: framebuffer.width,
        height: framebuffer.height,
        pitch: framebuffer.pitch,
        bits_per_pixel: framebuffer.bpp,
        red_mask_size: framebuffer.red_mask_size,
        red_mask_shift: framebuffer.red_mask_shift,
        green_mask_size: framebuffer.green_mask_size,
        green_mask_shift: framebuffer.green_mask_shift,
        blue_mask_size: framebuffer.blue_mask_size,
        blue_mask_shift: framebuffer.blue_mask_shift,
    };
    bootinfo_marker("[fb] framebuffer info return\r\n");
    Some(info)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SerialInfo {
    pub port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelImageInfo {
    pub sections: KernelSections,
    pub load_range: Option<KernelLoadRange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelLoadRange {
    pub physical_start: PhysicalAddress,
    pub virtual_start: VirtualAddress,
    pub length: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelSections {
    pub kernel: VirtualRange,
    pub text: VirtualRange,
    pub rodata: VirtualRange,
    pub data: VirtualRange,
    pub bss: VirtualRange,
}

impl KernelSections {
    #[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
    pub fn from_linker() -> Self {
        Self {
            kernel: VirtualRange::from_symbols(addr_of!(__kernel_start), addr_of!(__kernel_end)),
            text: VirtualRange::from_symbols(addr_of!(__text_start), addr_of!(__text_end)),
            rodata: VirtualRange::from_symbols(addr_of!(__rodata_start), addr_of!(__rodata_end)),
            data: VirtualRange::from_symbols(addr_of!(__data_start), addr_of!(__data_end)),
            bss: VirtualRange::from_symbols(addr_of!(__bss_start), addr_of!(__bss_end)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualRange {
    pub start: VirtualAddress,
    pub end: VirtualAddress,
}

impl VirtualRange {
    #[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
    fn from_symbols(start: *const u8, end: *const u8) -> Self {
        Self {
            start: VirtualAddress(start as u64),
            end: VirtualAddress(end as u64),
        }
    }

    pub const fn length(self) -> u64 {
        self.end.0.saturating_sub(self.start.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalAddress(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualAddress(pub u64);

#[cfg(test)]
mod tests {
    use super::*;

    fn boot_info_with_limine_base_revision(base_revision_supported: bool) -> BootInfo {
        BootInfo {
            boot_protocol: BootProtocol::Limine {
                base_revision_supported,
            },
            bootloader: BootloaderInfo::unknown(),
            memory_map: None,
            framebuffer: None,
            serial: None,
            rsdp: None,
            hhdm_offset: None,
            kernel: KernelImageInfo {
                sections: KernelSections {
                    kernel: VirtualRange {
                        start: VirtualAddress(0),
                        end: VirtualAddress(0),
                    },
                    text: VirtualRange {
                        start: VirtualAddress(0),
                        end: VirtualAddress(0),
                    },
                    rodata: VirtualRange {
                        start: VirtualAddress(0),
                        end: VirtualAddress(0),
                    },
                    data: VirtualRange {
                        start: VirtualAddress(0),
                        end: VirtualAddress(0),
                    },
                    bss: VirtualRange {
                        start: VirtualAddress(0),
                        end: VirtualAddress(0),
                    },
                },
                load_range: None,
            },
            modules: BootModules::empty(),
        }
    }

    #[test]
    fn limine_base_revision_helper_reflects_boot_protocol_field() {
        assert!(boot_info_with_limine_base_revision(true).limine_base_revision_supported());
        assert!(!boot_info_with_limine_base_revision(false).limine_base_revision_supported());
    }

    #[test]
    fn boot_string_from_cstr_null_pointer_returns_empty() {
        let boot_string = BootString::from_cstr(core::ptr::null());
        assert_eq!(boot_string, BootString::empty());
        assert_eq!(boot_string.as_bytes(), b"");
    }

    #[test]
    fn boot_string_bounded_scan_stops_at_nul() {
        static VALUE: &[u8] = b"Mirage\0ignored";
        let boot_string = BootString::from_cstr(VALUE.as_ptr());
        assert_eq!(boot_string.as_bytes(), b"Mirage");
    }

    #[test]
    fn boot_modules_empty_construction() {
        assert!(BootModules::empty().is_empty());
        assert!(BootModules::from_limine_response(None).is_empty());
    }

    #[test]
    fn memory_map_empty_construction() {
        assert!(MemoryMap::empty().is_empty());
        assert_eq!(MemoryMap::from_limine_response(None), None);
    }

    #[test]
    fn memory_map_entry_out_of_bounds_returns_none() {
        let entry = LimineMemoryMapEntry {
            base: 0x1000,
            length: 0x2000,
            entry_type: 0,
        };
        let entry_ptrs = [&entry as *const LimineMemoryMapEntry];
        let map = MemoryMap {
            entries: entry_ptrs.as_ptr(),
            entry_count: 1,
        };

        assert_eq!(map.entry(1), None);
    }

    #[test]
    fn first_framebuffer_missing_response_returns_none() {
        assert_eq!(first_framebuffer(None), None);
    }
}
