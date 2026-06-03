//! Read-only Limine boot-module block backend.

use crate::arch::x86_64::boot::BootModules;
use crate::kernel::device::{BlockStorageDevice, DeviceDriver, DeviceError, DeviceKind};
use crate::kernel::sync::SpinLock;
use crate::subkernel::{DeviceSecurity, SecurityClass};

#[derive(Clone, Copy)]
struct LimineBlockState {
    base: *const u8,
    size: usize,
}

unsafe impl Send for LimineBlockState {}

pub struct LimineModuleBlockDriver {
    state: SpinLock<LimineBlockState>,
}

impl LimineModuleBlockDriver {
    pub const SECTOR_SIZE: usize = 512;

    pub const fn new() -> Self {
        Self {
            state: SpinLock::new(LimineBlockState {
                base: core::ptr::null(),
                size: 0,
            }),
        }
    }

    pub fn configure_from_modules(&self, modules: BootModules) -> bool {
        let mut index = 0u64;
        while index < modules.len() {
            if let Some(module) = modules.module(index) {
                if module.size >= Self::SECTOR_SIZE as u64 {
                    *self.state.lock() = LimineBlockState {
                        base: module.base.0 as *const u8,
                        size: module.size as usize,
                    };
                    return true;
                }
            }
            index += 1;
        }
        false
    }

    pub fn is_configured(&self) -> bool {
        let state = self.state.lock();
        !state.base.is_null() && state.size >= Self::SECTOR_SIZE
    }

    fn validate_transfer(&self, first_sector: u64, byte_len: usize) -> Result<usize, DeviceError> {
        if byte_len % Self::SECTOR_SIZE != 0 {
            return Err(DeviceError::BufferTooSmall);
        }
        let start = (first_sector as usize)
            .checked_mul(Self::SECTOR_SIZE)
            .ok_or(DeviceError::Unsupported)?;
        let end = start
            .checked_add(byte_len)
            .ok_or(DeviceError::Unsupported)?;
        let state = self.state.lock();
        if state.base.is_null() || end > state.size {
            return Err(DeviceError::NotFound);
        }
        Ok(start)
    }
}

impl DeviceDriver for LimineModuleBlockDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::BlockStorage
    }

    fn name(&self) -> &'static str {
        "limine-module-block0"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Internal, false)
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        self.read_sectors(0, buffer)
    }

    fn as_block_storage(&self) -> Option<&dyn BlockStorageDevice> {
        Some(self)
    }
}

impl BlockStorageDevice for LimineModuleBlockDriver {
    fn sector_size(&self) -> usize {
        Self::SECTOR_SIZE
    }

    fn sector_count(&self) -> u64 {
        (self.state.lock().size / Self::SECTOR_SIZE) as u64
    }

    fn read_sectors(&self, first_sector: u64, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        let start = self.validate_transfer(first_sector, buffer.len())?;
        let state = self.state.lock();
        let src = unsafe { core::slice::from_raw_parts(state.base.add(start), buffer.len()) };
        buffer.copy_from_slice(src);
        Ok(buffer.len())
    }

    fn write_sectors(&self, _first_sector: u64, _data: &[u8]) -> Result<usize, DeviceError> {
        Err(DeviceError::Unsupported)
    }

    fn flush(&self) -> Result<(), DeviceError> {
        Ok(())
    }

    fn discard(&self, _first_sector: u64, _sector_count: u64) -> Result<(), DeviceError> {
        Err(DeviceError::Unsupported)
    }
}

pub static LIMINE_MODULE_BLOCK_DRIVER: LimineModuleBlockDriver = LimineModuleBlockDriver::new();
