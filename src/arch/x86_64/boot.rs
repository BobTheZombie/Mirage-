//! Earliest x86_64 bootstrap and typed boot information handoff.
//!
//! Limine enters the kernel in 64-bit mode with paging enabled. The assembly entry below still
//! takes ownership of the initial stack contract, aligns it to the SysV x86_64 ABI, clears the
//! kernel `.bss`, snapshots boot-protocol state, and only then calls the Rust kernel entry point.

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
use core::arch::global_asm;
#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
use core::ptr::addr_of;
#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
use core::ptr::write_bytes;

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
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
    call __mirage_x86_64_bootstrap
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

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
extern "Rust" {
    fn kernel_main(boot_info: BootInfo) -> !;
}

/// Architecture-owned entry point called by the `_start` assembly stub.
#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
#[no_mangle]
pub unsafe extern "C" fn __mirage_x86_64_bootstrap() -> ! {
    clear_bss();

    let sections = KernelSections::from_linker();
    let raw_boot = limine::snapshot();
    let boot_info = BootInfo::from_limine(raw_boot, sections);

    kernel_main(boot_info)
}

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
unsafe fn clear_bss() {
    let start = addr_of!(__bss_start) as usize;
    let end = addr_of!(__bss_end) as usize;
    let len = end.saturating_sub(start);

    if len != 0 {
        write_bytes(start as *mut u8, 0, len);
    }
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
    fn from_limine(raw: limine::LimineBootSnapshot, sections: KernelSections) -> Self {
        let executable = raw.executable_address.map(|address| KernelLoadRange {
            physical_start: PhysicalAddress(address.physical_base),
            virtual_start: VirtualAddress(address.virtual_base),
            length: sections.kernel.length(),
        });

        Self {
            boot_protocol: BootProtocol::Limine {
                base_revision_supported: raw.base_revision_supported,
            },
            bootloader: raw
                .bootloader
                .map(|info| BootloaderInfo {
                    name: BootString::from_cstr(info.name),
                    version: BootString::from_cstr(info.version),
                })
                .unwrap_or(BootloaderInfo::unknown()),
            memory_map: raw.memory_map.map(|map| MemoryMap {
                entries: map.entries,
                entry_count: map.entry_count,
            }),
            framebuffer: first_framebuffer(raw.framebuffer),
            serial: None,
            rsdp: raw.rsdp.map(|rsdp| PhysicalAddress(rsdp.address)),
            hhdm_offset: raw.hhdm.map(|hhdm| hhdm.offset),
            kernel: KernelImageInfo {
                sections,
                load_range: executable,
            },
            modules: raw
                .modules
                .map(|modules| BootModules {
                    entries: modules.modules,
                    entry_count: modules.module_count,
                })
                .unwrap_or(BootModules::empty()),
        }
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
        if ptr.is_null() {
            return Self::empty();
        }

        let mut len = 0;
        while len < 256 {
            let byte = unsafe { ptr.add(len).read() };
            if byte == 0 {
                break;
            }
            len += 1;
        }

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

    pub fn module(self, index: u64) -> Option<BootModule> {
        if index >= self.entry_count || self.entries.is_null() {
            return None;
        }
        let file_ptr = unsafe { self.entries.add(index as usize).read() };
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
    pub const fn len(self) -> u64 {
        self.entry_count
    }

    pub const fn is_empty(self) -> bool {
        self.entry_count == 0
    }

    pub fn entry(self, index: u64) -> Option<MemoryMapEntry> {
        if index >= self.entry_count || self.entries.is_null() {
            return None;
        }

        let entry_ptr = unsafe { self.entries.add(index as usize).read() };
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

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
fn first_framebuffer(
    response: Option<&'static limine::FramebufferResponse>,
) -> Option<FramebufferInfo> {
    let response = response?;
    if response.framebuffer_count == 0 || response.framebuffers.is_null() {
        return None;
    }

    let framebuffer_ptr = unsafe { response.framebuffers.read() };
    let framebuffer: &Framebuffer = unsafe { framebuffer_ptr.as_ref()? };
    Some(FramebufferInfo {
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
    })
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
    fn from_linker() -> Self {
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
}
