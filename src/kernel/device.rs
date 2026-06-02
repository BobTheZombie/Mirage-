use core::cmp::min;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::x86_64::boot::FramebufferInfo;
use crate::kernel::sync::SpinLock;
use crate::subkernel::{DeviceSecurity, SecurityClass};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceId(u16);

impl DeviceId {
    pub const fn new(id: u16) -> Self {
        Self(id)
    }

    pub const fn raw(&self) -> u16 {
        self.0
    }
}

/// Maximum number of bytes copied into a C-compatible device name.
pub const DEVICE_DESCRIPTOR_NAME_CAPACITY: usize = 32;

pub const MIRAGE_DEVICE_KIND_SERIAL_CONSOLE: u32 = 1;
pub const MIRAGE_DEVICE_KIND_SYSTEM_TIMER: u32 = 2;
pub const MIRAGE_DEVICE_KIND_BLOCK_STORAGE: u32 = 3;
pub const MIRAGE_DEVICE_KIND_FRAMEBUFFER: u32 = 4;
pub const MIRAGE_DEVICE_KIND_GPU_CAPABILITY: u32 = 5;
pub const MIRAGE_DEVICE_KIND_NETWORK_INTERFACE: u32 = 6;
pub const MIRAGE_DEVICE_KIND_INPUT_CONTROLLER: u32 = 7;
pub const MIRAGE_DEVICE_KIND_SUBKERNEL_CONTROL: u32 = 8;

pub const MIRAGE_SECURITY_CLASS_PUBLIC: u32 = 0;
pub const MIRAGE_SECURITY_CLASS_INTERNAL: u32 = 1;
pub const MIRAGE_SECURITY_CLASS_CONFIDENTIAL: u32 = 2;
pub const MIRAGE_SECURITY_CLASS_SYSTEM: u32 = 3;

pub const MIRAGE_DEVICE_FLAG_REQUIRES_KERNEL_MODE: u32 = 0b0001;

/// Stable C ABI representation of a device descriptor.
///
/// This intentionally uses only fixed-width integer fields and an inline,
/// NUL-padded UTF-8 name buffer so user-facing syscall and C wrapper ABIs do
/// not depend on Rust enum layouts or Rust string references.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirageDeviceDescriptor {
    pub id: u16,
    pub kind: u32,
    pub security_class: u32,
    pub flags: u32,
    pub name_len: u16,
    pub reserved: [u8; 6],
    pub name: [u8; DEVICE_DESCRIPTOR_NAME_CAPACITY],
}

impl MirageDeviceDescriptor {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            kind: 0,
            security_class: MIRAGE_SECURITY_CLASS_PUBLIC,
            flags: 0,
            name_len: 0,
            reserved: [0; 6],
            name: [0; DEVICE_DESCRIPTOR_NAME_CAPACITY],
        }
    }

    pub fn from_descriptor(descriptor: DeviceDescriptor) -> Self {
        let mut out = Self::empty();
        out.id = descriptor.id.raw();
        out.kind = encode_device_kind(descriptor.kind);
        out.security_class = encode_security_class(descriptor.class());
        if descriptor.requires_kernel_mode() {
            out.flags |= MIRAGE_DEVICE_FLAG_REQUIRES_KERNEL_MODE;
        }

        let bytes = descriptor.name.as_bytes();
        let copy_len = min(bytes.len(), DEVICE_DESCRIPTOR_NAME_CAPACITY);
        out.name[..copy_len].copy_from_slice(&bytes[..copy_len]);
        out.name_len = copy_len as u16;
        out
    }
}

impl Default for MirageDeviceDescriptor {
    fn default() -> Self {
        Self::empty()
    }
}

fn encode_device_kind(kind: DeviceKind) -> u32 {
    match kind {
        DeviceKind::SerialConsole => MIRAGE_DEVICE_KIND_SERIAL_CONSOLE,
        DeviceKind::SystemTimer => MIRAGE_DEVICE_KIND_SYSTEM_TIMER,
        DeviceKind::BlockStorage => MIRAGE_DEVICE_KIND_BLOCK_STORAGE,
        DeviceKind::Framebuffer => MIRAGE_DEVICE_KIND_FRAMEBUFFER,
        DeviceKind::GpuCapability => MIRAGE_DEVICE_KIND_GPU_CAPABILITY,
        DeviceKind::NetworkInterface => MIRAGE_DEVICE_KIND_NETWORK_INTERFACE,
        DeviceKind::InputController => MIRAGE_DEVICE_KIND_INPUT_CONTROLLER,
        DeviceKind::SubkernelControl => MIRAGE_DEVICE_KIND_SUBKERNEL_CONTROL,
    }
}

fn encode_security_class(class: SecurityClass) -> u32 {
    match class {
        SecurityClass::Public => MIRAGE_SECURITY_CLASS_PUBLIC,
        SecurityClass::Internal => MIRAGE_SECURITY_CLASS_INTERNAL,
        SecurityClass::Confidential => MIRAGE_SECURITY_CLASS_CONFIDENTIAL,
        SecurityClass::System => MIRAGE_SECURITY_CLASS_SYSTEM,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    SerialConsole,
    SystemTimer,
    BlockStorage,
    Framebuffer,
    GpuCapability,
    NetworkInterface,
    InputController,
    SubkernelControl,
}

/// Stable C ABI framebuffer metadata returned by the built-in framebuffer driver.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirageFramebufferDescriptor {
    pub address: u64,
    pub width: u64,
    pub height: u64,
    pub pitch: u64,
    pub bytes: u64,
    pub bits_per_pixel: u16,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
    pub flags: u32,
    pub reserved: [u8; 12],
}

impl MirageFramebufferDescriptor {
    pub const FLAG_PRESENT: u32 = 0b0001;

    pub const fn empty() -> Self {
        Self {
            address: 0,
            width: 0,
            height: 0,
            pitch: 0,
            bytes: 0,
            bits_per_pixel: 0,
            red_mask_size: 0,
            red_mask_shift: 0,
            green_mask_size: 0,
            green_mask_shift: 0,
            blue_mask_size: 0,
            blue_mask_shift: 0,
            flags: 0,
            reserved: [0; 12],
        }
    }

    pub const fn from_boot_framebuffer(framebuffer: FramebufferInfo) -> Self {
        Self {
            address: framebuffer.address.0,
            width: framebuffer.width,
            height: framebuffer.height,
            pitch: framebuffer.pitch,
            bytes: framebuffer.pitch.saturating_mul(framebuffer.height),
            bits_per_pixel: framebuffer.bits_per_pixel,
            red_mask_size: framebuffer.red_mask_size,
            red_mask_shift: framebuffer.red_mask_shift,
            green_mask_size: framebuffer.green_mask_size,
            green_mask_shift: framebuffer.green_mask_shift,
            blue_mask_size: framebuffer.blue_mask_size,
            blue_mask_shift: framebuffer.blue_mask_shift,
            flags: Self::FLAG_PRESENT,
            reserved: [0; 12],
        }
    }
}

/// Stable C ABI GPU capability metadata used by display-facing L2 daemons.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirageGpuCapabilityDescriptor {
    pub flags: u32,
    pub framebuffer_count: u32,
    pub max_width: u64,
    pub max_height: u64,
    pub preferred_width: u64,
    pub preferred_height: u64,
    pub preferred_bits_per_pixel: u16,
    pub reserved: [u8; 14],
}

impl MirageGpuCapabilityDescriptor {
    pub const FLAG_LINEAR_FRAMEBUFFER: u32 = 0b0001;

    pub const fn empty() -> Self {
        Self {
            flags: 0,
            framebuffer_count: 0,
            max_width: 0,
            max_height: 0,
            preferred_width: 0,
            preferred_height: 0,
            preferred_bits_per_pixel: 0,
            reserved: [0; 14],
        }
    }

    pub const fn from_framebuffer(framebuffer: MirageFramebufferDescriptor) -> Self {
        if framebuffer.flags & MirageFramebufferDescriptor::FLAG_PRESENT == 0 {
            return Self::empty();
        }
        Self {
            flags: Self::FLAG_LINEAR_FRAMEBUFFER,
            framebuffer_count: 1,
            max_width: framebuffer.width,
            max_height: framebuffer.height,
            preferred_width: framebuffer.width,
            preferred_height: framebuffer.height,
            preferred_bits_per_pixel: framebuffer.bits_per_pixel,
            reserved: [0; 14],
        }
    }
}

/// Stable C ABI network interface metadata for the built-in loopback/null NIC.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirageNetworkInterfaceDescriptor {
    pub mtu: u32,
    pub flags: u32,
    pub mac: [u8; 6],
    pub reserved: [u8; 10],
}

impl MirageNetworkInterfaceDescriptor {
    pub const FLAG_UP: u32 = 0b0001;
    pub const FLAG_LOOPBACK: u32 = 0b0010;
    pub const FLAG_NULL_BACKEND: u32 = 0b0100;

    pub const fn loopback() -> Self {
        Self {
            mtu: 65_536,
            flags: Self::FLAG_UP | Self::FLAG_LOOPBACK | Self::FLAG_NULL_BACKEND,
            mac: [0, 0, 0, 0, 0, 0],
            reserved: [0; 10],
        }
    }
}

/// Stable C ABI input controller metadata for the built-in empty input queue.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirageInputControllerDescriptor {
    pub event_size: u16,
    pub queue_capacity: u16,
    pub flags: u32,
    pub pending_events: u32,
    pub reserved: [u8; 20],
}

impl MirageInputControllerDescriptor {
    pub const FLAG_EMPTY_QUEUE: u32 = 0b0001;

    pub const fn empty_queue() -> Self {
        Self {
            event_size: core::mem::size_of::<MirageInputEvent>() as u16,
            queue_capacity: 0,
            flags: Self::FLAG_EMPTY_QUEUE,
            pending_events: 0,
            reserved: [0; 20],
        }
    }
}

/// Stable C ABI input event payload. The built-in driver currently has no events.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirageInputEvent {
    pub event_type: u16,
    pub code: u16,
    pub value: i32,
    pub timestamp_ns: u64,
}

fn copy_c_abi_metadata<T>(metadata: &T, buffer: &mut [u8]) -> Result<usize, DeviceError> {
    let len = core::mem::size_of::<T>();
    if buffer.len() < len {
        return Err(DeviceError::BufferTooSmall);
    }
    let bytes = unsafe { core::slice::from_raw_parts(metadata as *const T as *const u8, len) };
    buffer[..len].copy_from_slice(bytes);
    Ok(len)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceDescriptor {
    pub id: DeviceId,
    pub kind: DeviceKind,
    pub name: &'static str,
    pub security: DeviceSecurity,
}

impl DeviceDescriptor {
    pub const fn new(
        id: DeviceId,
        kind: DeviceKind,
        name: &'static str,
        security: DeviceSecurity,
    ) -> Self {
        Self {
            id,
            kind,
            name,
            security,
        }
    }

    pub const fn class(&self) -> SecurityClass {
        self.security.class()
    }

    pub const fn requires_kernel_mode(&self) -> bool {
        self.security.requires_kernel_mode()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceError {
    NotFound,
    RegistryFull,
    BufferTooSmall,
    Unsupported,
    Busy,
}

pub trait DeviceDriver {
    fn kind(&self) -> DeviceKind;
    fn name(&self) -> &'static str;
    fn security(&self) -> DeviceSecurity;
    fn read(&self, _buffer: &mut [u8]) -> Result<usize, DeviceError> {
        Err(DeviceError::Unsupported)
    }
    fn write(&self, _data: &[u8]) -> Result<usize, DeviceError> {
        Err(DeviceError::Unsupported)
    }
    fn as_block_storage(&self) -> Option<&dyn BlockStorageDevice> {
        None
    }
}

/// Sector-addressed interface implemented by block storage devices.
///
/// Block filesystems should use these operations instead of treating storage as
/// a byte stream: every read or write starts at a logical sector and transfers
/// whole sectors whose byte length is `sector_size() * sector_count`.
pub trait BlockStorageDevice {
    fn sector_size(&self) -> usize;
    fn sector_count(&self) -> u64;
    fn read_sectors(&self, first_sector: u64, buffer: &mut [u8]) -> Result<usize, DeviceError>;
    fn write_sectors(&self, first_sector: u64, data: &[u8]) -> Result<usize, DeviceError>;
    fn flush(&self) -> Result<(), DeviceError>;
    fn discard(&self, first_sector: u64, sector_count: u64) -> Result<(), DeviceError>;
    fn write_zeroes(&self, first_sector: u64, sector_count: u64) -> Result<(), DeviceError> {
        self.discard(first_sector, sector_count)
    }
}

#[derive(Clone, Copy)]
struct DeviceEntry {
    id: DeviceId,
    driver: &'static dyn DeviceDriver,
}

impl DeviceEntry {
    fn descriptor(&self) -> DeviceDescriptor {
        DeviceDescriptor::new(
            self.id,
            self.driver.kind(),
            self.driver.name(),
            self.driver.security(),
        )
    }
}

pub struct DeviceManager<const MAX: usize> {
    devices: [Option<DeviceEntry>; MAX],
    next_id: u16,
}

impl<const MAX: usize> DeviceManager<MAX> {
    pub const fn new() -> Self {
        Self {
            devices: [None; MAX],
            next_id: 1,
        }
    }

    pub fn reset(&mut self) {
        self.next_id = 1;
        let mut idx = 0;
        while idx < MAX {
            self.devices[idx] = None;
            idx += 1;
        }
    }

    pub fn install_core_devices(&mut self) {
        self.install_core_devices_with_framebuffer(None);
    }

    pub fn install_core_devices_with_framebuffer(&mut self, framebuffer: Option<FramebufferInfo>) {
        configure_graphics_devices(framebuffer);
        let _ = self.register_driver(&SERIAL_CONSOLE_DRIVER);
        let _ = self.register_driver(&SYSTEM_TIMER_DRIVER);
        let _ = self.register_driver(&BLOCK_STORAGE_DRIVER);
        let _ = self.register_driver(&FRAMEBUFFER_DRIVER);
        let _ = self.register_driver(&GPU_CAPABILITY_DRIVER);
        let _ = self.register_driver(&LOOPBACK_NETWORK_DRIVER);
        let _ = self.register_driver(&INPUT_CONTROLLER_DRIVER);
    }

    pub fn register_driver(
        &mut self,
        driver: &'static dyn DeviceDriver,
    ) -> Result<DeviceDescriptor, DeviceError> {
        let slot = self.find_free_slot().ok_or(DeviceError::RegistryFull)?;
        let id = DeviceId::new(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.devices[slot] = Some(DeviceEntry { id, driver });
        Ok(self.devices[slot].unwrap().descriptor())
    }

    pub fn descriptor(&self, id: DeviceId) -> Option<DeviceDescriptor> {
        self.find_device(id).map(|entry| entry.descriptor())
    }

    pub fn enumerate(&self, out: &mut [DeviceDescriptor]) -> usize {
        let mut count = 0usize;
        let mut idx = 0usize;
        while idx < MAX {
            if let Some(entry) = self.devices[idx] {
                if count < out.len() {
                    out[count] = entry.descriptor();
                    count += 1;
                } else {
                    break;
                }
            }
            idx += 1;
        }
        count
    }

    pub fn read(&self, id: DeviceId, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        let entry = self.find_device(id).ok_or(DeviceError::NotFound)?;
        entry.driver.read(buffer)
    }

    pub fn write(&self, id: DeviceId, data: &[u8]) -> Result<usize, DeviceError> {
        let entry = self.find_device(id).ok_or(DeviceError::NotFound)?;
        entry.driver.write(data)
    }

    pub fn block_storage(&self, id: DeviceId) -> Result<&dyn BlockStorageDevice, DeviceError> {
        let entry = self.find_device(id).ok_or(DeviceError::NotFound)?;
        if entry.driver.kind() != DeviceKind::BlockStorage {
            return Err(DeviceError::Unsupported);
        }
        entry
            .driver
            .as_block_storage()
            .ok_or(DeviceError::Unsupported)
    }

    pub fn sector_size(&self, id: DeviceId) -> Result<usize, DeviceError> {
        Ok(self.block_storage(id)?.sector_size())
    }

    pub fn sector_count(&self, id: DeviceId) -> Result<u64, DeviceError> {
        Ok(self.block_storage(id)?.sector_count())
    }

    pub fn read_sectors(
        &self,
        id: DeviceId,
        first_sector: u64,
        buffer: &mut [u8],
    ) -> Result<usize, DeviceError> {
        self.block_storage(id)?.read_sectors(first_sector, buffer)
    }

    pub fn write_sectors(
        &self,
        id: DeviceId,
        first_sector: u64,
        data: &[u8],
    ) -> Result<usize, DeviceError> {
        self.block_storage(id)?.write_sectors(first_sector, data)
    }

    pub fn flush_block_storage(&self, id: DeviceId) -> Result<(), DeviceError> {
        self.block_storage(id)?.flush()
    }

    pub fn discard_sectors(
        &self,
        id: DeviceId,
        first_sector: u64,
        sector_count: u64,
    ) -> Result<(), DeviceError> {
        self.block_storage(id)?.discard(first_sector, sector_count)
    }

    pub fn write_zeroes(
        &self,
        id: DeviceId,
        first_sector: u64,
        sector_count: u64,
    ) -> Result<(), DeviceError> {
        self.block_storage(id)?
            .write_zeroes(first_sector, sector_count)
    }

    fn find_free_slot(&self) -> Option<usize> {
        let mut idx = 0usize;
        while idx < MAX {
            if self.devices[idx].is_none() {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    fn find_device(&self, id: DeviceId) -> Option<DeviceEntry> {
        let mut idx = 0usize;
        while idx < MAX {
            if let Some(entry) = self.devices[idx] {
                if entry.id.raw() == id.raw() {
                    return Some(entry);
                }
            }
            idx += 1;
        }
        None
    }
}

struct SerialBuffer {
    data: [u8; SerialConsoleDriver::CAPACITY],
    len: usize,
}

impl SerialBuffer {
    const fn new() -> Self {
        Self {
            data: [0; SerialConsoleDriver::CAPACITY],
            len: 0,
        }
    }

    fn push(&mut self, payload: &[u8]) -> usize {
        let space = SerialConsoleDriver::CAPACITY - self.len;
        let to_copy = min(space, payload.len());
        if to_copy == 0 {
            return 0;
        }
        self.data[self.len..self.len + to_copy].copy_from_slice(&payload[..to_copy]);
        self.len += to_copy;
        to_copy
    }

    fn pop(&mut self, buffer: &mut [u8]) -> usize {
        let to_copy = min(self.len, buffer.len());
        if to_copy == 0 {
            return 0;
        }
        buffer[..to_copy].copy_from_slice(&self.data[..to_copy]);
        let remaining = self.len - to_copy;
        let mut idx = 0usize;
        while idx < remaining {
            self.data[idx] = self.data[idx + to_copy];
            idx += 1;
        }
        self.len = remaining;
        to_copy
    }
}

pub struct SerialConsoleDriver {
    buffer: SpinLock<SerialBuffer>,
}

impl SerialConsoleDriver {
    const CAPACITY: usize = 256;

    pub const fn new() -> Self {
        Self {
            buffer: SpinLock::new(SerialBuffer::new()),
        }
    }
}

impl DeviceDriver for SerialConsoleDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::SerialConsole
    }

    fn name(&self) -> &'static str {
        "serial-console"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Internal, false)
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        let mut state = self.buffer.lock();
        Ok(state.pop(buffer))
    }

    fn write(&self, data: &[u8]) -> Result<usize, DeviceError> {
        let mut state = self.buffer.lock();
        Ok(state.push(data))
    }
}

pub struct SystemTimerDriver {
    ticks: AtomicU64,
}

impl SystemTimerDriver {
    pub const fn new() -> Self {
        Self {
            ticks: AtomicU64::new(0),
        }
    }

    pub fn tick(&self) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
    }
}

impl DeviceDriver for SystemTimerDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::SystemTimer
    }

    fn name(&self) -> &'static str {
        "system-timer"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::System, true)
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        if buffer.len() < core::mem::size_of::<u64>() {
            return Err(DeviceError::BufferTooSmall);
        }
        let value = self.ticks.load(Ordering::Relaxed);
        let bytes = value.to_le_bytes();
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }
}

struct BlockStorageState {
    sectors: [[u8; BlockStorageDriver::SECTOR_SIZE]; BlockStorageDriver::SECTOR_COUNT],
}

impl BlockStorageState {
    const fn new() -> Self {
        Self {
            sectors: [[0; BlockStorageDriver::SECTOR_SIZE]; BlockStorageDriver::SECTOR_COUNT],
        }
    }
}

/// Built-in RAM-backed block device used until platform storage drivers register
/// their own block devices with [`DeviceManager`].
pub struct BlockStorageDriver {
    state: SpinLock<BlockStorageState>,
}

impl BlockStorageDriver {
    pub const SECTOR_SIZE: usize = 512;
    pub const SECTOR_COUNT: usize = 160;

    pub const fn new() -> Self {
        Self {
            state: SpinLock::new(BlockStorageState::new()),
        }
    }

    fn validate_transfer(&self, first_sector: u64, byte_len: usize) -> Result<usize, DeviceError> {
        let sector_size = Self::SECTOR_SIZE;
        if byte_len % sector_size != 0 {
            return Err(DeviceError::BufferTooSmall);
        }
        let sectors = byte_len / sector_size;
        let last_sector = first_sector
            .checked_add(sectors as u64)
            .ok_or(DeviceError::Unsupported)?;
        if first_sector > Self::SECTOR_COUNT as u64 || last_sector > Self::SECTOR_COUNT as u64 {
            return Err(DeviceError::NotFound);
        }
        Ok(sectors)
    }
}

impl DeviceDriver for BlockStorageDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::BlockStorage
    }

    fn name(&self) -> &'static str {
        "ram-block0"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Confidential, false)
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        self.read_sectors(0, buffer)
    }

    fn write(&self, data: &[u8]) -> Result<usize, DeviceError> {
        self.write_sectors(0, data)
    }

    fn as_block_storage(&self) -> Option<&dyn BlockStorageDevice> {
        Some(self)
    }
}

impl BlockStorageDevice for BlockStorageDriver {
    fn sector_size(&self) -> usize {
        Self::SECTOR_SIZE
    }

    fn sector_count(&self) -> u64 {
        Self::SECTOR_COUNT as u64
    }

    fn read_sectors(&self, first_sector: u64, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        let sectors = self.validate_transfer(first_sector, buffer.len())?;
        let state = self.state.lock();
        let mut idx = 0usize;
        while idx < sectors {
            let sector = first_sector as usize + idx;
            let start = idx * Self::SECTOR_SIZE;
            let end = start + Self::SECTOR_SIZE;
            buffer[start..end].copy_from_slice(&state.sectors[sector]);
            idx += 1;
        }
        Ok(buffer.len())
    }

    fn write_sectors(&self, first_sector: u64, data: &[u8]) -> Result<usize, DeviceError> {
        let sectors = self.validate_transfer(first_sector, data.len())?;
        let mut state = self.state.lock();
        let mut idx = 0usize;
        while idx < sectors {
            let sector = first_sector as usize + idx;
            let start = idx * Self::SECTOR_SIZE;
            let end = start + Self::SECTOR_SIZE;
            state.sectors[sector].copy_from_slice(&data[start..end]);
            idx += 1;
        }
        Ok(data.len())
    }

    fn flush(&self) -> Result<(), DeviceError> {
        Ok(())
    }

    fn discard(&self, first_sector: u64, sector_count: u64) -> Result<(), DeviceError> {
        let byte_len = (sector_count as usize)
            .checked_mul(Self::SECTOR_SIZE)
            .ok_or(DeviceError::Unsupported)?;
        let sectors = self.validate_transfer(first_sector, byte_len)?;
        let mut state = self.state.lock();
        let mut idx = 0usize;
        while idx < sectors {
            state.sectors[first_sector as usize + idx].fill(0);
            idx += 1;
        }
        Ok(())
    }

    fn write_zeroes(&self, first_sector: u64, sector_count: u64) -> Result<(), DeviceError> {
        self.discard(first_sector, sector_count)
    }
}

pub struct FramebufferDriver {
    descriptor: SpinLock<MirageFramebufferDescriptor>,
}

impl FramebufferDriver {
    pub const fn new() -> Self {
        Self {
            descriptor: SpinLock::new(MirageFramebufferDescriptor::empty()),
        }
    }

    fn configure(&self, framebuffer: Option<FramebufferInfo>) {
        let mut descriptor = self.descriptor.lock();
        *descriptor = framebuffer
            .map(MirageFramebufferDescriptor::from_boot_framebuffer)
            .unwrap_or(MirageFramebufferDescriptor::empty());
    }
}

impl DeviceDriver for FramebufferDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::Framebuffer
    }

    fn name(&self) -> &'static str {
        "limine-framebuffer0"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Internal, false)
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        let descriptor = self.descriptor.lock();
        copy_c_abi_metadata(&*descriptor, buffer)
    }
}

pub struct GpuCapabilityDriver {
    descriptor: SpinLock<MirageGpuCapabilityDescriptor>,
}

impl GpuCapabilityDriver {
    pub const fn new() -> Self {
        Self {
            descriptor: SpinLock::new(MirageGpuCapabilityDescriptor::empty()),
        }
    }

    fn configure(&self, framebuffer: Option<FramebufferInfo>) {
        let framebuffer = framebuffer
            .map(MirageFramebufferDescriptor::from_boot_framebuffer)
            .unwrap_or(MirageFramebufferDescriptor::empty());
        let mut descriptor = self.descriptor.lock();
        *descriptor = MirageGpuCapabilityDescriptor::from_framebuffer(framebuffer);
    }
}

impl DeviceDriver for GpuCapabilityDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::GpuCapability
    }

    fn name(&self) -> &'static str {
        "gpu-capability0"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Internal, false)
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        let descriptor = self.descriptor.lock();
        copy_c_abi_metadata(&*descriptor, buffer)
    }
}

pub struct LoopbackNetworkDriver;

impl LoopbackNetworkDriver {
    pub const fn new() -> Self {
        Self
    }
}

impl DeviceDriver for LoopbackNetworkDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::NetworkInterface
    }

    fn name(&self) -> &'static str {
        "loopback-net0"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Internal, false)
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        copy_c_abi_metadata(&MirageNetworkInterfaceDescriptor::loopback(), buffer)
    }

    fn write(&self, data: &[u8]) -> Result<usize, DeviceError> {
        Ok(data.len())
    }
}

pub struct InputControllerDriver;

impl InputControllerDriver {
    pub const fn new() -> Self {
        Self
    }
}

impl DeviceDriver for InputControllerDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::InputController
    }

    fn name(&self) -> &'static str {
        "input-controller0"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Internal, false)
    }

    fn read(&self, _buffer: &mut [u8]) -> Result<usize, DeviceError> {
        Ok(0)
    }
}

pub fn configure_graphics_devices(framebuffer: Option<FramebufferInfo>) {
    FRAMEBUFFER_DRIVER.configure(framebuffer);
    GPU_CAPABILITY_DRIVER.configure(framebuffer);
}

static SERIAL_CONSOLE_DRIVER: SerialConsoleDriver = SerialConsoleDriver::new();
static SYSTEM_TIMER_DRIVER: SystemTimerDriver = SystemTimerDriver::new();
static BLOCK_STORAGE_DRIVER: BlockStorageDriver = BlockStorageDriver::new();
static FRAMEBUFFER_DRIVER: FramebufferDriver = FramebufferDriver::new();
static GPU_CAPABILITY_DRIVER: GpuCapabilityDriver = GpuCapabilityDriver::new();
static LOOPBACK_NETWORK_DRIVER: LoopbackNetworkDriver = LoopbackNetworkDriver::new();
static INPUT_CONTROLLER_DRIVER: InputControllerDriver = InputControllerDriver::new();

pub fn system_timer() -> &'static SystemTimerDriver {
    &SYSTEM_TIMER_DRIVER
}

pub const fn built_in_block_storage() -> &'static BlockStorageDriver {
    &BLOCK_STORAGE_DRIVER
}
