use core::cmp::min;
use core::sync::atomic::{AtomicU64, Ordering};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    SerialConsole,
    SystemTimer,
    BlockStorage,
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
        let _ = self.register_driver(&SERIAL_CONSOLE_DRIVER);
        let _ = self.register_driver(&SYSTEM_TIMER_DRIVER);
        let _ = self.register_driver(&BLOCK_STORAGE_DRIVER);
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
    block: [u8; BlockStorageDriver::BLOCK_SIZE],
}

impl BlockStorageState {
    const fn new() -> Self {
        Self {
            block: [0; BlockStorageDriver::BLOCK_SIZE],
        }
    }
}

pub struct BlockStorageDriver {
    state: SpinLock<BlockStorageState>,
}

impl BlockStorageDriver {
    const BLOCK_SIZE: usize = 512;

    pub const fn new() -> Self {
        Self {
            state: SpinLock::new(BlockStorageState::new()),
        }
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
        if buffer.len() < Self::BLOCK_SIZE {
            return Err(DeviceError::BufferTooSmall);
        }
        let state = self.state.lock();
        buffer[..Self::BLOCK_SIZE].copy_from_slice(&state.block);
        Ok(Self::BLOCK_SIZE)
    }

    fn write(&self, data: &[u8]) -> Result<usize, DeviceError> {
        let mut state = self.state.lock();
        let to_copy = min(Self::BLOCK_SIZE, data.len());
        state.block[..to_copy].copy_from_slice(&data[..to_copy]);
        if to_copy < Self::BLOCK_SIZE {
            let mut idx = to_copy;
            while idx < Self::BLOCK_SIZE {
                state.block[idx] = 0;
                idx += 1;
            }
        }
        Ok(to_copy)
    }
}

static SERIAL_CONSOLE_DRIVER: SerialConsoleDriver = SerialConsoleDriver::new();
static SYSTEM_TIMER_DRIVER: SystemTimerDriver = SystemTimerDriver::new();
static BLOCK_STORAGE_DRIVER: BlockStorageDriver = BlockStorageDriver::new();

pub fn system_timer() -> &'static SystemTimerDriver {
    &SYSTEM_TIMER_DRIVER
}
