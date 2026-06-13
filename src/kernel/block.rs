//! Generic no-heap block layer used by early storage discovery.
//!
//! The registry intentionally stores static driver objects only.  AHCI/NVMe
//! discovery owns hardware setup; this layer owns stable names, IDs, bounds
//! validation, and partition-device registration without allocating.

pub type BlockDeviceId = u32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockDeviceKind {
    SataDisk,
    AtapiOptical,
    NvmeNamespace,
    RamDisk,
    BuiltInQfs,
    Partition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockDeviceInfo {
    pub id: BlockDeviceId,
    pub name: &'static str,
    pub kind: BlockDeviceKind,
    pub block_size: u32,
    pub block_count: u64,
    pub readonly: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockError {
    NotFound,
    RegistryFull,
    DuplicateName,
    InvalidBlockSize,
    InvalidBufferLength,
    OutOfBounds,
    ReadOnly,
    Unsupported,
    Timeout,
    Io,
    NoMedia,
    Crc,
    InvalidSignature,
}

pub trait BlockDevice: Sync {
    fn info(&self) -> BlockDeviceInfo;

    fn read_blocks(&self, lba: u64, count: u32, buffer: &mut [u8]) -> Result<(), BlockError>;

    fn write_blocks(&self, lba: u64, count: u32, buffer: &[u8]) -> Result<(), BlockError>;

    fn flush(&self) -> Result<(), BlockError>;
}

#[derive(Clone, Copy)]
struct Entry {
    info: BlockDeviceInfo,
    device: &'static dyn BlockDevice,
}

pub struct BlockRegistry<const N: usize> {
    entries: [Option<Entry>; N],
    next_id: BlockDeviceId,
}

impl<const N: usize> BlockRegistry<N> {
    pub const fn new() -> Self {
        Self {
            entries: [None; N],
            next_id: 1,
        }
    }

    pub fn reset(&mut self) {
        self.entries = [None; N];
        self.next_id = 1;
    }

    pub fn register_device(
        &mut self,
        device: &'static dyn BlockDevice,
    ) -> Result<BlockDeviceId, BlockError> {
        let mut info = device.info();
        validate_geometry(info)?;
        if self.lookup_by_name(info.name).is_some() {
            return Err(BlockError::DuplicateName);
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(BlockError::RegistryFull)?;
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        info.id = id;
        *slot = Some(Entry { info, device });
        Ok(id)
    }

    pub fn unregister_device(&mut self, id: BlockDeviceId) -> Result<(), BlockError> {
        for entry in &mut self.entries {
            if entry.map(|candidate| candidate.info.id) == Some(id) {
                *entry = None;
                return Ok(());
            }
        }
        Err(BlockError::NotFound)
    }

    pub fn lookup_by_id(&self, id: BlockDeviceId) -> Option<BlockDeviceInfo> {
        self.entries
            .iter()
            .flatten()
            .find(|entry| entry.info.id == id)
            .map(|entry| entry.info)
    }

    pub fn lookup_by_name(&self, name: &str) -> Option<BlockDeviceInfo> {
        self.entries
            .iter()
            .flatten()
            .find(|entry| entry.info.name == name)
            .map(|entry| entry.info)
    }

    pub fn device_by_name(&self, name: &str) -> Option<&'static dyn BlockDevice> {
        self.entries
            .iter()
            .flatten()
            .find(|entry| entry.info.name == name)
            .map(|entry| entry.device)
    }

    pub fn enumerate(&self, out: &mut [BlockDeviceInfo]) -> usize {
        let mut count = 0;
        for entry in self.entries.iter().flatten() {
            if count == out.len() {
                break;
            }
            out[count] = entry.info;
            count += 1;
        }
        count
    }
}

pub fn validate_geometry(info: BlockDeviceInfo) -> Result<(), BlockError> {
    if info.block_size == 0 || info.block_count == 0 {
        return Err(BlockError::InvalidBlockSize);
    }
    Ok(())
}

pub fn validate_transfer(
    info: BlockDeviceInfo,
    lba: u64,
    count: u32,
    buffer_len: usize,
) -> Result<usize, BlockError> {
    if count == 0
        || lba
            .checked_add(count as u64)
            .map_or(true, |end| end > info.block_count)
    {
        return Err(BlockError::OutOfBounds);
    }
    let bytes = (count as usize)
        .checked_mul(info.block_size as usize)
        .ok_or(BlockError::InvalidBufferLength)?;
    if buffer_len != bytes {
        return Err(BlockError::InvalidBufferLength);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MemDev(&'static str);

    impl BlockDevice for MemDev {
        fn info(&self) -> BlockDeviceInfo {
            BlockDeviceInfo {
                id: 0,
                name: self.0,
                kind: BlockDeviceKind::RamDisk,
                block_size: 512,
                block_count: 4,
                readonly: true,
            }
        }

        fn read_blocks(&self, lba: u64, count: u32, buffer: &mut [u8]) -> Result<(), BlockError> {
            validate_transfer(self.info(), lba, count, buffer.len()).map(|_| ())
        }

        fn write_blocks(&self, _lba: u64, _count: u32, _buffer: &[u8]) -> Result<(), BlockError> {
            Err(BlockError::ReadOnly)
        }

        fn flush(&self) -> Result<(), BlockError> {
            Ok(())
        }
    }

    static DEV0: MemDev = MemDev("ram0");
    static DEV0_DUP: MemDev = MemDev("ram0");

    #[test]
    fn block_registry_rejects_duplicate_names_and_keeps_stable_ids() {
        let mut registry: BlockRegistry<2> = BlockRegistry::new();
        assert_eq!(registry.register_device(&DEV0), Ok(1));
        assert_eq!(
            registry.register_device(&DEV0_DUP),
            Err(BlockError::DuplicateName)
        );
        assert_eq!(registry.lookup_by_name("ram0").unwrap().id, 1);
        assert_eq!(registry.lookup_by_id(1).unwrap().name, "ram0");
    }

    #[test]
    fn block_bounds_validation_rejects_bad_ranges_and_lengths() {
        let info = DEV0.info();
        assert_eq!(validate_transfer(info, 0, 1, 512), Ok(512));
        assert_eq!(
            validate_transfer(info, 4, 1, 512),
            Err(BlockError::OutOfBounds)
        );
        assert_eq!(
            validate_transfer(info, 0, 1, 256),
            Err(BlockError::InvalidBufferLength)
        );
    }
}
