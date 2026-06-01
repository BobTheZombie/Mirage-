//! Earliest x86_64 bootstrap and typed boot information handoff.
//!
//! Limine enters the kernel in 64-bit mode with paging enabled. The assembly entry below still
//! takes ownership of the initial stack contract, aligns it to the SysV x86_64 ABI, clears the
//! kernel `.bss`, snapshots boot-protocol state, and only then calls the Rust kernel entry point.

#[cfg(not(test))]
use core::arch::global_asm;
#[cfg(not(test))]
use core::ptr::addr_of;
#[cfg(not(test))]
use core::ptr::write_bytes;

use crate::boot::MemoryMapEntry as LimineMemoryMapEntry;
#[cfg(not(test))]
use crate::boot::{self as limine, Framebuffer};

#[cfg(not(test))]
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

#[cfg(not(test))]
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

#[cfg(not(test))]
extern "Rust" {
    fn kernel_main(boot_info: BootInfo) -> !;
}

/// Architecture-owned entry point called by the `_start` assembly stub.
#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn __mirage_x86_64_bootstrap() -> ! {
    clear_bss();

    let sections = KernelSections::from_linker();
    let raw_boot = limine::snapshot();
    let boot_info = BootInfo::from_limine(raw_boot, sections);

    kernel_main(boot_info)
}

#[cfg(not(test))]
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
}

impl BootInfo {
    #[cfg(not(test))]
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

#[cfg(not(test))]
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
    #[cfg(not(test))]
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
    #[cfg(not(test))]
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
