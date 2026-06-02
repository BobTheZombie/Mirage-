//! Userspace QFS image operations backed by host [`std::fs::File`] handles.
//!
//! This module is compiled only when the `qfs-std` Cargo feature is enabled so
//! hosted tooling can format and inspect QFS images without depending on the
//! private kernel filesystem module layout.

use std::boxed::Box;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

use crate::kernel::device::{BlockStorageDevice, DeviceError};

pub use crate::kernel::fs::qfs::{
    QfsSuperblock, QFS_BOOK_PAGES, QFS_PAGE_SECTORS, QFS_SECTOR_SIZE,
};

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

    use super::*;

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
    use crate::kernel::fs::qfs::{
        QfsBookHeader, QfsBookIndexEntry, QfsBookRole, QfsInodeRecord, QfsPageLocation,
        QfsSuperblock, QFS_BOOK_INDEX_ENTRY_BYTES, QFS_BOOK_PAGES, QFS_MAX_INODE_RECORDS,
        QFS_PAGE_SECTORS,
    };

    let book_size_sectors = QFS_BOOK_PAGES as u64 * QFS_PAGE_SECTORS as u64;
    if sector_count < 1 + book_size_sectors {
        return Err(QfsToolError::InvalidArgument(
            "QFS images must contain at least one complete book",
        ));
    }

    let total_books = ((sector_count - 1) / book_size_sectors)
        .try_into()
        .map_err(|_| QfsToolError::InvalidArgument("QFS image is too large"))?;
    let formatted_sectors = 1 + u64::from(total_books) * book_size_sectors;
    let reserved_sectors = 1 + 1 + 1 + u64::from(QFS_PAGE_SECTORS) * 3;
    let free_sectors = formatted_sectors.saturating_sub(reserved_sectors);

    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    let device = StdQfsBlockDevice::create_sized(file, QFS_SECTOR_SIZE, formatted_sectors)?;

    let superblock = QfsSuperblock {
        total_books,
        root_inode: crate::kernel::fs::inode::InodeId::ROOT.raw(),
        inode_table: QfsPageLocation::new(0, 1),
        free_space_bitmap: QfsPageLocation::new(0, 2),
        journal: QfsPageLocation::new(0, 8),
        total_sectors: formatted_sectors,
        free_sectors,
        ..QfsSuperblock::empty()
    };

    let mut sector = [0u8; QFS_SECTOR_SIZE];
    superblock.write_sector(&mut sector)?;
    device.write_sectors(0, &sector)?;

    let mut header = QfsBookHeader::empty();
    header.book_id = 0;
    header.chapter_count = 3;
    header.index_entry_count = 3;
    sector.fill(0);
    header.write_sector(&mut sector)?;
    device.write_sectors(1, &sector)?;

    sector.fill(0);
    QfsBookIndexEntry {
        chapter_id: 1,
        first_page: 1,
        page_count: 1,
        role: QfsBookRole::Inode,
        flags: 0,
        reserved: [0; 6],
    }
    .write(&mut sector[0..QFS_BOOK_INDEX_ENTRY_BYTES])?;
    QfsBookIndexEntry {
        chapter_id: 2,
        first_page: 2,
        page_count: 1,
        role: QfsBookRole::FreeMap,
        flags: 0,
        reserved: [0; 6],
    }
    .write(&mut sector[QFS_BOOK_INDEX_ENTRY_BYTES..QFS_BOOK_INDEX_ENTRY_BYTES * 2])?;
    QfsBookIndexEntry {
        chapter_id: 3,
        first_page: 8,
        page_count: QFS_BOOK_PAGES - 8,
        role: QfsBookRole::Journal,
        flags: 0,
        reserved: [0; 6],
    }
    .write(&mut sector[QFS_BOOK_INDEX_ENTRY_BYTES * 2..QFS_BOOK_INDEX_ENTRY_BYTES * 3])?;
    device.write_sectors(2, &sector)?;

    let mut inodes = [None; QFS_MAX_INODE_RECORDS];
    inodes[0] = Some(QfsInodeRecord::root());
    sector.fill(0);
    crate::kernel::fs::qfs::serialize_inode_records(&inodes, &mut sector)?;
    device.write_sectors(1 + u64::from(QFS_PAGE_SECTORS), &sector)?;
    device.flush()?;

    Ok(QfsFormatReport {
        sector_count: formatted_sectors,
        total_books,
        free_sectors,
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
    let mut sector = [0u8; QFS_SECTOR_SIZE];
    crate::kernel::device::BlockStorageDevice::read_sectors(&device, 0, &mut sector)?;
    Ok(QfsSuperblock::parse_sector(&sector)?)
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
    Ok(QfsStatReport { image, inode })
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
    let mut sector = [0u8; QFS_SECTOR_SIZE];
    device.read_sectors(0, &mut sector)?;
    Ok(QfsSuperblock::parse_sector(&sector)?)
}
