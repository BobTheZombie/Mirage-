//! Userspace QFS image operations backed by generic Mirage block/storage handles.
//!
//! This module is compiled only when the `qfs-std` Cargo feature is enabled so
//! hosted tooling can format and inspect QFS images without depending on the
//! private kernel filesystem module layout or driver-specific storage crates.

use std::boxed::Box;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;
use std::vec;

use crate::kernel::device::{BlockStorageDevice, DeviceError};
use mirage_block::{BlockDevice, BlockDeviceInfo, BlockError, BlockRange, Lba, SectorCount};
use mirage_storage::{StorageCapability, StorageDeviceHandle, StorageError, StorageService};

pub use crate::kernel::fs::qfs_format::{
    QfsSuperblock, QFS_BOOK_PAGES, QFS_PAGE_SECTORS, QFS_SECTOR_SIZE,
};

/// QFS sector adapter over Mirage's backend-independent [`mirage_block::BlockDevice`] trait.
///
/// Driver services such as NVMe, AHCI, USB storage, or future transports should expose
/// devices through `mirage-block`; QFS consumes only this stable block abstraction.
pub struct MirageBlockQfsDevice<D: BlockDevice> {
    device: Mutex<D>,
    info: BlockDeviceInfo,
}

impl<D: BlockDevice> MirageBlockQfsDevice<D> {
    /// Wraps any `mirage-block` device behind QFS's existing sector-addressed API.
    pub fn new(device: D) -> Result<Self, DeviceError> {
        let info = device.info();
        validate_sector_size(info.block_size.bytes_usize())?;
        Ok(Self {
            device: Mutex::new(device),
            info,
        })
    }

    fn range_for_transfer(
        &self,
        first_sector: u64,
        byte_count: usize,
    ) -> Result<Option<BlockRange>, DeviceError> {
        let sector_size = self.sector_size();
        if byte_count % sector_size != 0 {
            return Err(DeviceError::BufferTooSmall);
        }

        let sectors = byte_count / sector_size;
        if sectors == 0 {
            return Ok(None);
        }

        let sector_count = u64::try_from(sectors).map_err(|_| DeviceError::Unsupported)?;
        Ok(Some(BlockRange::new(
            Lba::new(first_sector),
            SectorCount::new(sector_count),
        )))
    }
}

impl<D: BlockDevice> BlockStorageDevice for MirageBlockQfsDevice<D> {
    fn sector_size(&self) -> usize {
        self.info.block_size.bytes_usize()
    }

    fn sector_count(&self) -> u64 {
        self.info.sectors.get()
    }

    fn read_sectors(&self, first_sector: u64, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        let Some(range) = self.range_for_transfer(first_sector, buffer.len())? else {
            return Ok(0);
        };
        let mut device = self.device.lock().map_err(|_| DeviceError::Busy)?;
        device.read_blocks(range, buffer).map_err(map_block_error)?;
        Ok(buffer.len())
    }

    fn write_sectors(&self, first_sector: u64, data: &[u8]) -> Result<usize, DeviceError> {
        let Some(range) = self.range_for_transfer(first_sector, data.len())? else {
            return Ok(0);
        };
        let mut device = self.device.lock().map_err(|_| DeviceError::Busy)?;
        device.write_blocks(range, data).map_err(map_block_error)?;
        Ok(data.len())
    }

    fn flush(&self) -> Result<(), DeviceError> {
        let mut device = self.device.lock().map_err(|_| DeviceError::Busy)?;
        device.flush().map_err(map_block_error)
    }

    fn discard(&self, first_sector: u64, sector_count: u64) -> Result<(), DeviceError> {
        self.write_zeroes(first_sector, sector_count)
    }

    fn write_zeroes(&self, first_sector: u64, sector_count: u64) -> Result<(), DeviceError> {
        if sector_count == 0 {
            return Ok(());
        }

        let byte_count = byte_len(self.sector_size(), sector_count)?;
        let data_len = usize::try_from(byte_count).map_err(|_| DeviceError::Unsupported)?;
        let zeroes = vec![0u8; data_len];
        self.write_sectors(first_sector, &zeroes)?;
        Ok(())
    }
}

/// Capability-checked QFS adapter over a supervisor-owned [`mirage_storage::StorageService`].
///
/// This is the preferred service-kernel path for mounting QFS from a storage handle:
/// the filesystem sees only sector operations, while access remains mediated by the
/// storage capability issued for the registered generic block device.
pub struct StorageServiceQfsDevice {
    service: Mutex<StorageService>,
    handle: StorageDeviceHandle,
    capability: StorageCapability,
}

impl StorageServiceQfsDevice {
    /// Wraps a registered storage handle and capability for QFS sector I/O.
    pub fn new(
        service: StorageService,
        handle: StorageDeviceHandle,
        capability: StorageCapability,
    ) -> Result<Self, DeviceError> {
        validate_sector_size(handle.info().block_size.bytes_usize())?;
        if capability.device_id() != handle.id() {
            return Err(DeviceError::Unsupported);
        }
        Ok(Self {
            service: Mutex::new(service),
            handle,
            capability,
        })
    }

    fn range_for_transfer(
        &self,
        first_sector: u64,
        byte_count: usize,
    ) -> Result<Option<BlockRange>, DeviceError> {
        let sector_size = self.sector_size();
        if byte_count % sector_size != 0 {
            return Err(DeviceError::BufferTooSmall);
        }

        let sectors = byte_count / sector_size;
        if sectors == 0 {
            return Ok(None);
        }

        let sector_count = u64::try_from(sectors).map_err(|_| DeviceError::Unsupported)?;
        Ok(Some(BlockRange::new(
            Lba::new(first_sector),
            SectorCount::new(sector_count),
        )))
    }
}

impl BlockStorageDevice for StorageServiceQfsDevice {
    fn sector_size(&self) -> usize {
        self.handle.info().block_size.bytes_usize()
    }

    fn sector_count(&self) -> u64 {
        self.handle.info().sectors.get()
    }

    fn read_sectors(&self, first_sector: u64, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        let Some(range) = self.range_for_transfer(first_sector, buffer.len())? else {
            return Ok(0);
        };
        let mut service = self.service.lock().map_err(|_| DeviceError::Busy)?;
        service
            .read_blocks(&self.capability, &self.handle, range, buffer)
            .map_err(map_storage_error)?;
        Ok(buffer.len())
    }

    fn write_sectors(&self, first_sector: u64, data: &[u8]) -> Result<usize, DeviceError> {
        let Some(range) = self.range_for_transfer(first_sector, data.len())? else {
            return Ok(0);
        };
        let mut service = self.service.lock().map_err(|_| DeviceError::Busy)?;
        service
            .write_blocks(&self.capability, &self.handle, range, data)
            .map_err(map_storage_error)?;
        Ok(data.len())
    }

    fn flush(&self) -> Result<(), DeviceError> {
        let mut service = self.service.lock().map_err(|_| DeviceError::Busy)?;
        service
            .flush(&self.capability, &self.handle)
            .map_err(map_storage_error)
    }

    fn discard(&self, first_sector: u64, sector_count: u64) -> Result<(), DeviceError> {
        self.write_zeroes(first_sector, sector_count)
    }

    fn write_zeroes(&self, first_sector: u64, sector_count: u64) -> Result<(), DeviceError> {
        if sector_count == 0 {
            return Ok(());
        }

        let byte_count = byte_len(self.sector_size(), sector_count)?;
        let data_len = usize::try_from(byte_count).map_err(|_| DeviceError::Unsupported)?;
        let zeroes = vec![0u8; data_len];
        self.write_sectors(first_sector, &zeroes)?;
        Ok(())
    }
}

/// A host-side block device for QFS disk images backed by [`std::fs::File`].
///
/// The adapter is intended for integration tests and tooling that mount QFS
/// against a regular host file. All I/O is serialized through an internal
/// mutex because [`BlockStorageDevice`] methods take `&self`.
pub struct StdQfsBlockDevice {
    file: Mutex<File>,
    sector_size: usize,
    sector_count: u64,
}

impl StdQfsBlockDevice {
    /// Creates an adapter over an existing QFS image file.
    ///
    /// The file length must be an exact multiple of the QFS logical sector
    /// size. Use [`Self::create_sized`] when a test needs to resize a freshly
    /// created image before mounting it.
    pub fn new(file: File) -> Result<Self, DeviceError> {
        Self::with_sector_size(file, QFS_SECTOR_SIZE)
    }

    /// Creates an adapter over an existing image with an explicit sector size.
    pub fn with_sector_size(file: File, sector_size: usize) -> Result<Self, DeviceError> {
        let len = file.metadata().map_err(map_std_error)?.len();
        let sector_count = sector_count_from_len(len, sector_size)?;
        Ok(Self {
            file: Mutex::new(file),
            sector_size,
            sector_count,
        })
    }

    /// Resizes `file` to `sector_size * sector_count` bytes and returns an
    /// adapter over the resulting image.
    pub fn create_sized(
        file: File,
        sector_size: usize,
        sector_count: u64,
    ) -> Result<Self, DeviceError> {
        validate_sector_size(sector_size)?;
        let len = byte_len(sector_size, sector_count)?;
        file.set_len(len).map_err(map_std_error)?;
        Ok(Self {
            file: Mutex::new(file),
            sector_size,
            sector_count,
        })
    }

    fn validate_transfer(&self, first_sector: u64, byte_count: usize) -> Result<u64, DeviceError> {
        if byte_count % self.sector_size != 0 {
            return Err(DeviceError::BufferTooSmall);
        }

        let sectors = (byte_count / self.sector_size) as u64;
        let last_sector = first_sector
            .checked_add(sectors)
            .ok_or(DeviceError::Unsupported)?;
        if first_sector > self.sector_count || last_sector > self.sector_count {
            return Err(DeviceError::NotFound);
        }
        Ok(sectors)
    }

    fn offset_for_sector(&self, first_sector: u64) -> Result<u64, DeviceError> {
        byte_len(self.sector_size, first_sector)
    }
}

impl BlockStorageDevice for StdQfsBlockDevice {
    fn sector_size(&self) -> usize {
        self.sector_size
    }

    fn sector_count(&self) -> u64 {
        self.sector_count
    }

    fn read_sectors(&self, first_sector: u64, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        self.validate_transfer(first_sector, buffer.len())?;
        let offset = self.offset_for_sector(first_sector)?;
        let mut file = self.file.lock().map_err(|_| DeviceError::Busy)?;
        file.seek(SeekFrom::Start(offset)).map_err(map_std_error)?;
        file.read_exact(buffer).map_err(map_std_error)?;
        Ok(buffer.len())
    }

    fn write_sectors(&self, first_sector: u64, data: &[u8]) -> Result<usize, DeviceError> {
        self.validate_transfer(first_sector, data.len())?;
        let offset = self.offset_for_sector(first_sector)?;
        let mut file = self.file.lock().map_err(|_| DeviceError::Busy)?;
        file.seek(SeekFrom::Start(offset)).map_err(map_std_error)?;
        file.write_all(data).map_err(map_std_error)?;
        Ok(data.len())
    }

    fn flush(&self) -> Result<(), DeviceError> {
        let file = self.file.lock().map_err(|_| DeviceError::Busy)?;
        file.sync_all().map_err(map_std_error)
    }

    fn discard(&self, first_sector: u64, sector_count: u64) -> Result<(), DeviceError> {
        self.write_zeroes(first_sector, sector_count)
    }

    fn write_zeroes(&self, first_sector: u64, sector_count: u64) -> Result<(), DeviceError> {
        let byte_count = byte_len(self.sector_size, sector_count)?;
        self.validate_transfer(
            first_sector,
            usize::try_from(byte_count).map_err(|_| DeviceError::Unsupported)?,
        )?;
        let offset = self.offset_for_sector(first_sector)?;
        let mut file = self.file.lock().map_err(|_| DeviceError::Busy)?;
        file.seek(SeekFrom::Start(offset)).map_err(map_std_error)?;

        let zeroes = [0u8; QFS_SECTOR_SIZE];
        let mut remaining = byte_count;
        while remaining > 0 {
            let chunk = core::cmp::min(remaining, zeroes.len() as u64) as usize;
            file.write_all(&zeroes[..chunk]).map_err(map_std_error)?;
            remaining -= chunk as u64;
        }
        Ok(())
    }
}

fn validate_sector_size(sector_size: usize) -> Result<(), DeviceError> {
    if sector_size == 0 {
        return Err(DeviceError::Unsupported);
    }
    Ok(())
}

fn sector_count_from_len(len: u64, sector_size: usize) -> Result<u64, DeviceError> {
    validate_sector_size(sector_size)?;
    let sector_size = sector_size as u64;
    if len % sector_size != 0 {
        return Err(DeviceError::BufferTooSmall);
    }
    Ok(len / sector_size)
}

fn byte_len(sector_size: usize, sector_count: u64) -> Result<u64, DeviceError> {
    validate_sector_size(sector_size)?;
    (sector_size as u64)
        .checked_mul(sector_count)
        .ok_or(DeviceError::Unsupported)
}

fn map_block_error(error: BlockError) -> DeviceError {
    match error {
        BlockError::OutOfBounds => DeviceError::NotFound,
        BlockError::BufferSizeMismatch | BlockError::EmptyRange | BlockError::InvalidBlockSize => {
            DeviceError::BufferTooSmall
        }
        BlockError::DeviceOffline | BlockError::DeviceFaulted | BlockError::QueueEmpty => {
            DeviceError::Busy
        }
        BlockError::RangeOverflow
        | BlockError::ReadOnly
        | BlockError::DeviceMismatch
        | BlockError::Io => DeviceError::Unsupported,
    }
}

fn map_storage_error(error: StorageError) -> DeviceError {
    match error {
        StorageError::DeviceNotFound => DeviceError::NotFound,
        StorageError::AccessDenied | StorageError::CapabilityRevoked => DeviceError::Unsupported,
        StorageError::DeviceAlreadyRegistered => DeviceError::Busy,
        StorageError::Block(error) => map_block_error(error),
    }
}

fn map_std_error(error: std::io::Error) -> DeviceError {
    match error.kind() {
        std::io::ErrorKind::NotFound => DeviceError::NotFound,
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted => DeviceError::Busy,
        std::io::ErrorKind::UnexpectedEof => DeviceError::BufferTooSmall,
        _ => DeviceError::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::path::PathBuf;
    use std::vec;
    use std::vec::Vec;

    use super::*;
    use crate::kernel::fs::path::Path;
    use crate::kernel::fs::vfs::FileSystem;
    use mirage_block::{BlockDeviceId, BlockDeviceState, BlockSize};

    struct MockBlockDevice {
        info: BlockDeviceInfo,
        state: BlockDeviceState,
        storage: Vec<u8>,
        flushes: usize,
    }

    impl MockBlockDevice {
        fn new(id: u64, sectors: u64) -> Self {
            let block_size = BlockSize::new(QFS_SECTOR_SIZE as u32).unwrap();
            Self {
                info: BlockDeviceInfo::new(
                    BlockDeviceId::new(id),
                    block_size,
                    SectorCount::new(sectors),
                    false,
                    true,
                ),
                state: BlockDeviceState::Online,
                storage: vec![0; sectors as usize * block_size.bytes_usize()],
                flushes: 0,
            }
        }

        fn byte_bounds(&self, range: BlockRange) -> (usize, usize) {
            let start = range.start().get() as usize * self.info.block_size.bytes_usize();
            let len = range.byte_len(self.info.block_size).unwrap();
            (start, start + len)
        }
    }

    impl BlockDevice for MockBlockDevice {
        fn info(&self) -> BlockDeviceInfo {
            self.info
        }

        fn state(&self) -> BlockDeviceState {
            self.state
        }

        fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError> {
            self.validate_read(range, buffer)?;
            let (start, end) = self.byte_bounds(range);
            buffer.copy_from_slice(&self.storage[start..end]);
            Ok(())
        }

        fn write_blocks(&mut self, range: BlockRange, data: &[u8]) -> Result<(), BlockError> {
            self.validate_write(range, data)?;
            let (start, end) = self.byte_bounds(range);
            self.storage[start..end].copy_from_slice(data);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            self.state.ensure_available()?;
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn qfs_mounts_root_object_through_storage_service_handle() {
        let mut service = StorageService::new();
        let handle = service
            .register_device(Box::new(MockBlockDevice::new(42, 1024)))
            .unwrap();
        let capability = service.grant_read_write(handle.id()).unwrap();
        let adapter = StorageServiceQfsDevice::new(service, handle, capability).unwrap();

        crate::kernel::fs::qfs_format::initialize_image(&adapter, 1024).unwrap();
        adapter.flush().unwrap();

        let adapter: &'static StorageServiceQfsDevice = Box::leak(Box::new(adapter));
        let fs = crate::kernel::fs::qfs::QfsFileSystem::new_on_block_device(false, adapter);
        fs.refresh_from_block_device().unwrap();

        let root_path = Path::new("/").unwrap();
        let root_inode = fs.lookup(root_path).unwrap();
        let root_object = fs.lookup_object_record(root_path).unwrap();

        assert_eq!(root_inode.id, crate::kernel::fs::inode::InodeId::ROOT);
        assert_eq!(
            root_object.object_id,
            crate::kernel::fs::inode::InodeId::ROOT.raw()
        );
        assert_eq!(adapter.sector_size(), QFS_SECTOR_SIZE);
        assert_eq!(adapter.sector_count(), 1024);
    }

    #[test]
    fn reads_writes_flushes_and_zeroes_file_backed_sectors() {
        let path = test_image_path("rw_zeroes");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .expect("create qfs test image");
        let device = StdQfsBlockDevice::create_sized(file, QFS_SECTOR_SIZE, 4).unwrap();

        assert_eq!(device.sector_size(), QFS_SECTOR_SIZE);
        assert_eq!(device.sector_count(), 4);

        let data = [0x5au8; QFS_SECTOR_SIZE * 2];
        assert_eq!(device.write_sectors(1, &data).unwrap(), data.len());
        device.flush().unwrap();

        let mut read_back = [0u8; QFS_SECTOR_SIZE * 2];
        assert_eq!(
            device.read_sectors(1, &mut read_back).unwrap(),
            read_back.len()
        );
        assert_eq!(read_back, data);

        device.write_zeroes(1, 2).unwrap();
        device.read_sectors(1, &mut read_back).unwrap();
        assert_eq!(read_back, [0u8; QFS_SECTOR_SIZE * 2]);

        std::fs::remove_file(path).ok();
    }

    fn test_image_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "mirage_qfs_std_{name}_{}_{}.img",
            std::process::id(),
            unique_suffix()
        ));
        path
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}

/// Result of formatting a host-backed QFS image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsFormatReport {
    pub sector_count: u64,
    pub total_books: u32,
    pub free_sectors: u64,
}

/// Summary returned by host-side QFS inspection commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsImageReport {
    pub superblock: QfsSuperblock,
    pub cached_books: usize,
    pub cached_book_index_entries: usize,
    pub cached_chapter_index_entries: usize,
    pub cached_inode_records: usize,
    pub cached_journal_records: usize,
}

/// Metadata returned by host-side QFS `stat` operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsStatReport {
    pub image: QfsImageReport,
    pub inode: crate::kernel::fs::inode::InodeMetadata,
    pub object_id: u64,
    pub path_identity: u64,
    pub metadata_flags: u16,
    pub service_class: u16,
    pub extent_map_version: u32,
    pub extent_count: u16,
    pub signature_len: u16,
    pub capability_len: u16,
    pub last_transaction_id: u64,
    pub mutation_state: u16,
}

/// Error type used by hosted QFS tooling APIs.
#[derive(Debug)]
pub enum QfsToolError {
    Io(std::io::Error),
    Device(DeviceError),
    Vfs(crate::kernel::fs::vfs::VfsError),
    InvalidArgument(&'static str),
}

impl core::fmt::Display for QfsToolError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Device(error) => write!(formatter, "device error: {error:?}"),
            Self::Vfs(error) => write!(formatter, "VFS error: {error:?}"),
            Self::InvalidArgument(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for QfsToolError {}

impl From<std::io::Error> for QfsToolError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<DeviceError> for QfsToolError {
    fn from(error: DeviceError) -> Self {
        Self::Device(error)
    }
}

impl From<crate::kernel::fs::vfs::VfsError> for QfsToolError {
    fn from(error: crate::kernel::fs::vfs::VfsError) -> Self {
        Self::Vfs(error)
    }
}

/// Formats `path` as a minimal QFS image with a root inode and empty metadata tables.
pub fn mkfs_image<P: AsRef<std::path::Path>>(
    path: P,
    sector_count: u64,
) -> Result<QfsFormatReport, QfsToolError> {
    use crate::kernel::device::BlockStorageDevice;
    use crate::kernel::fs::qfs_format::initialize_image;

    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    let device = StdQfsBlockDevice::create_sized(file, QFS_SECTOR_SIZE, sector_count)?;
    let superblock = initialize_image(&device, sector_count)?;
    device.flush()?;

    Ok(QfsFormatReport {
        sector_count: superblock.total_sectors,
        total_books: superblock.total_books,
        free_sectors: superblock.free_sectors,
    })
}

/// Opens, replays the journal for, and validates a host-backed QFS image.
pub fn fsck_image<P: AsRef<std::path::Path>>(path: P) -> Result<QfsImageReport, QfsToolError> {
    let fs = open_userspace_qfs(path, false)?;
    fs.replay_journal()?;
    fs.refresh_from_block_device()?;
    image_report(&fs)
}

/// Reads the sector-zero QFS superblock from a host-backed image.
pub fn dump_superblock<P: AsRef<std::path::Path>>(path: P) -> Result<QfsSuperblock, QfsToolError> {
    let file = std::fs::OpenOptions::new().read(true).open(path)?;
    let device = StdQfsBlockDevice::new(file)?;
    Ok(crate::kernel::fs::qfs_format::read_superblock(&device)?)
}

/// Returns mounted QFS metadata plus a stat lookup for `path` inside the image.
pub fn stat_image<P: AsRef<std::path::Path>>(
    image_path: P,
    qfs_path: &str,
) -> Result<QfsStatReport, QfsToolError> {
    use crate::kernel::fs::vfs::FileSystem;

    let fs = open_userspace_qfs(image_path, true)?;
    let image = image_report(&fs)?;
    let path = crate::kernel::fs::path::Path::new(qfs_path)
        .map_err(crate::kernel::fs::vfs::VfsError::from)?;
    let inode = fs.lookup(path)?;
    let object = fs.lookup_object_record(path)?;
    Ok(QfsStatReport {
        image,
        inode,
        object_id: object.object_id,
        path_identity: object.path_identity,
        metadata_flags: object.metadata_flags,
        service_class: object.service_class,
        extent_map_version: object.extent_map_version,
        extent_count: object.extent_count,
        signature_len: object.signature_len,
        capability_len: object.capability_len,
        last_transaction_id: object.last_transaction_id,
        mutation_state: object.mutation_state,
    })
}

fn open_userspace_qfs<P: AsRef<std::path::Path>>(
    path: P,
    read_only: bool,
) -> Result<crate::kernel::fs::qfs::QfsFileSystem, QfsToolError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(!read_only)
        .open(path)?;
    let device = StdQfsBlockDevice::new(file)?;
    let device: &'static StdQfsBlockDevice = Box::leak(Box::new(device));
    let fs = crate::kernel::fs::qfs::QfsFileSystem::new_on_block_device(read_only, device);
    fs.refresh_from_block_device()?;
    Ok(fs)
}

fn image_report(
    fs: &crate::kernel::fs::qfs::QfsFileSystem,
) -> Result<QfsImageReport, QfsToolError> {
    use crate::kernel::fs::vfs::FileSystem;

    let superblock = fs.super_block();
    let (
        cached_books,
        cached_book_index_entries,
        cached_chapter_index_entries,
        cached_inode_records,
        cached_journal_records,
    ) = fs.cached_table_counts();
    Ok(QfsImageReport {
        superblock: QfsSuperblock {
            root_inode: superblock.root.raw(),
            sector_size: superblock.block_size as u16,
            total_sectors: superblock.total_blocks,
            free_sectors: superblock.free_blocks,
            ..dump_mounted_superblock(fs)?
        },
        cached_books,
        cached_book_index_entries,
        cached_chapter_index_entries,
        cached_inode_records,
        cached_journal_records,
    })
}

fn dump_mounted_superblock(
    fs: &crate::kernel::fs::qfs::QfsFileSystem,
) -> Result<QfsSuperblock, QfsToolError> {
    let device = fs.block_device().ok_or(QfsToolError::InvalidArgument(
        "QFS filesystem has no block device",
    ))?;
    Ok(crate::kernel::fs::qfs_format::read_superblock(device)?)
}
