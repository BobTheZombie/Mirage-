//! Boot protocol glue used by the hardware boot artifact.
//!
//! Mirage currently speaks the Limine boot protocol. The request records below are kept in a
//! linker-retained writable section so Limine can discover them in the ELF image, populate the
//! response pointers, and then jump to `_start` with paging and a stack already established.

use core::ptr::{addr_of, read_volatile};

const LIMINE_COMMON_MAGIC: [u64; 2] = [0xc7b1_dd30_df4c_8b88, 0x0a82_e883_a194_f07b];
const LIMINE_BASE_REVISION_MAGIC: [u64; 2] = [0xf956_2b2d_5c95_a6c8, 0x6a7b_3849_4453_6bdc];
const LIMINE_REQUESTS_START_MARKER: [u64; 4] = [
    0xf6b8_f4b3_9de7_d1ae,
    0xfab9_1a69_40fc_b9cf,
    0x785c_6ed0_15d3_e316,
    0x181e_920a_7852_b9d9,
];
const LIMINE_REQUESTS_END_MARKER: [u64; 2] = [0xadc0_e053_1bb1_0d03, 0x9572_709f_3176_4c62];

const BOOTLOADER_INFO_REQUEST: [u64; 4] =
    limine_request_id(0xf550_38d8_e2a1_202f, 0x2794_26fc_f5f5_9740);
const STACK_SIZE_REQUEST: [u64; 4] =
    limine_request_id(0x224e_f046_0a8e_8926, 0xe1cb_0fc2_5f46_ea3d);
const HHDM_REQUEST: [u64; 4] = limine_request_id(0x48dc_f1cb_8ad2_b852, 0x6398_4e95_9a98_244b);
const FRAMEBUFFER_REQUEST: [u64; 4] =
    limine_request_id(0x9d58_27dc_d881_dd75, 0xa314_8604_f6fa_b11b);
const MEMORY_MAP_REQUEST: [u64; 4] =
    limine_request_id(0x67cf_3d9d_378a_806f, 0xe304_acdf_c50c_3c62);
const RSDP_REQUEST: [u64; 4] = limine_request_id(0xc5e7_7b6b_397e_7b43, 0x2763_7845_accd_cf3c);
const EXECUTABLE_ADDRESS_REQUEST: [u64; 4] =
    limine_request_id(0x71ba_7686_3cc5_5f63, 0xb264_4a48_c516_a487);

const fn limine_request_id(kind0: u64, kind1: u64) -> [u64; 4] {
    [LIMINE_COMMON_MAGIC[0], LIMINE_COMMON_MAGIC[1], kind0, kind1]
}

#[repr(C)]
pub struct LimineRequest<T> {
    id: [u64; 4],
    revision: u64,
    response: *mut T,
}

impl<T> LimineRequest<T> {
    pub const fn new(id: [u64; 4]) -> Self {
        Self {
            id,
            revision: 0,
            response: core::ptr::null_mut(),
        }
    }

    pub fn response(&self) -> Option<&'static T> {
        let response = unsafe { read_volatile(addr_of!(self.response)) };
        unsafe { response.as_ref() }
    }
}

unsafe impl<T> Sync for LimineRequest<T> {}

#[repr(C)]
pub struct StackSizeRequest {
    id: [u64; 4],
    revision: u64,
    response: *mut StackSizeResponse,
    stack_size: u64,
}

impl StackSizeRequest {
    pub const fn new(stack_size: u64) -> Self {
        Self {
            id: STACK_SIZE_REQUEST,
            revision: 0,
            response: core::ptr::null_mut(),
            stack_size,
        }
    }
}

unsafe impl Sync for StackSizeRequest {}

#[repr(C)]
pub struct BootloaderInfoResponse {
    pub revision: u64,
    pub name: *const u8,
    pub version: *const u8,
}

#[repr(C)]
pub struct StackSizeResponse {
    pub revision: u64,
}

#[repr(C)]
pub struct HhdmResponse {
    pub revision: u64,
    pub offset: u64,
}

#[repr(C)]
pub struct FramebufferResponse {
    pub revision: u64,
    pub framebuffer_count: u64,
    pub framebuffers: *const *const Framebuffer,
}

#[repr(C)]
pub struct Framebuffer {
    pub address: *mut u8,
    pub width: u64,
    pub height: u64,
    pub pitch: u64,
    pub bpp: u16,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
    pub unused: [u8; 7],
    pub edid_size: u64,
    pub edid: *const u8,
    pub mode_count: u64,
    pub modes: *const *const VideoMode,
}

#[repr(C)]
pub struct VideoMode {
    pub pitch: u64,
    pub width: u64,
    pub height: u64,
    pub bpp: u16,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
}

#[repr(C)]
pub struct MemoryMapResponse {
    pub revision: u64,
    pub entry_count: u64,
    pub entries: *const *const MemoryMapEntry,
}

#[repr(C)]
pub struct MemoryMapEntry {
    pub base: u64,
    pub length: u64,
    pub entry_type: u64,
}

#[repr(C)]
pub struct RsdpResponse {
    pub revision: u64,
    pub address: u64,
}

#[repr(C)]
pub struct ExecutableAddressResponse {
    pub revision: u64,
    pub physical_base: u64,
    pub virtual_base: u64,
}

#[used]
#[link_section = ".requests_start_marker"]
static REQUESTS_START_MARKER: [u64; 4] = LIMINE_REQUESTS_START_MARKER;

#[used]
#[link_section = ".requests"]
static mut BASE_REVISION: [u64; 3] = [
    LIMINE_BASE_REVISION_MAGIC[0],
    LIMINE_BASE_REVISION_MAGIC[1],
    3,
];

#[used]
#[link_section = ".requests"]
pub static BOOTLOADER_INFO: LimineRequest<BootloaderInfoResponse> =
    LimineRequest::new(BOOTLOADER_INFO_REQUEST);

#[used]
#[link_section = ".requests"]
pub static STACK_SIZE: StackSizeRequest = StackSizeRequest::new(64 * 1024);

#[used]
#[link_section = ".requests"]
pub static HHDM: LimineRequest<HhdmResponse> = LimineRequest::new(HHDM_REQUEST);

#[used]
#[link_section = ".requests"]
pub static FRAMEBUFFER: LimineRequest<FramebufferResponse> =
    LimineRequest::new(FRAMEBUFFER_REQUEST);

#[used]
#[link_section = ".requests"]
pub static MEMORY_MAP: LimineRequest<MemoryMapResponse> = LimineRequest::new(MEMORY_MAP_REQUEST);

#[used]
#[link_section = ".requests"]
pub static RSDP: LimineRequest<RsdpResponse> = LimineRequest::new(RSDP_REQUEST);

#[used]
#[link_section = ".requests"]
pub static EXECUTABLE_ADDRESS: LimineRequest<ExecutableAddressResponse> =
    LimineRequest::new(EXECUTABLE_ADDRESS_REQUEST);

#[used]
#[link_section = ".requests_end_marker"]
static REQUESTS_END_MARKER: [u64; 2] = LIMINE_REQUESTS_END_MARKER;

/// Returns whether Limine accepted the requested base protocol revision.
pub fn base_revision_supported() -> bool {
    let revision_slot = unsafe { addr_of!(BASE_REVISION[2]) };
    unsafe { read_volatile(revision_slot) == 0 }
}

/// Capture the firmware-provided data that the early kernel can consume without allocation.
pub fn snapshot() -> LimineBootSnapshot {
    LimineBootSnapshot {
        base_revision_supported: base_revision_supported(),
        bootloader: BOOTLOADER_INFO.response(),
        hhdm: HHDM.response(),
        framebuffer: FRAMEBUFFER.response(),
        memory_map: MEMORY_MAP.response(),
        rsdp: RSDP.response(),
        executable_address: EXECUTABLE_ADDRESS.response(),
    }
}

#[derive(Clone, Copy)]
pub struct LimineBootSnapshot {
    pub base_revision_supported: bool,
    pub bootloader: Option<&'static BootloaderInfoResponse>,
    pub hhdm: Option<&'static HhdmResponse>,
    pub framebuffer: Option<&'static FramebufferResponse>,
    pub memory_map: Option<&'static MemoryMapResponse>,
    pub rsdp: Option<&'static RsdpResponse>,
    pub executable_address: Option<&'static ExecutableAddressResponse>,
}
