//! Host/testing QFS block-device adapter backed by [`std::fs::File`].
//!
//! This module is compiled only when the `qfs-std` Cargo feature is enabled so
//! the default kernel build does not import or link the Rust standard library.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

use crate::kernel::device::{BlockStorageDevice, DeviceError};

use super::qfs::QFS_SECTOR_SIZE;

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
