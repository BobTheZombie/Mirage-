//! no_std ext4-derived backend primitives for Mirage block filesystems.
//!
//! This module deliberately keeps the on-disk vocabulary close to ext4 while
//! avoiding heap allocation and host-endian layout assumptions.  Parsers and
//! serializers consume caller-provided byte slices, fixed arrays, and const
//! generic tables so the code can be used by early kernel mount code, recovery
//! paths, and small USB/SSD boot volumes.

use core::{cmp::min, str};

use crate::kernel::{
    device::{BlockStorageDevice, DeviceError},
    fs::{
        file::{File, OpenFlags},
        inode::{DirEntry, InodeId, InodeKind, InodeMetadata},
        path::Path,
        permissions::{AccessMode, Credentials, Permissions},
        vfs::{FileSystem, SuperBlock as VfsSuperBlock, VfsError},
    },
};

pub const EXT4_SUPERBLOCK_OFFSET: u64 = 1024;
pub const EXT4_SUPERBLOCK_SIZE: usize = 1024;
pub const EXT4_SUPER_MAGIC: u16 = 0xef53;
pub const EXT4_GOOD_OLD_INODE_SIZE: u16 = 128;
pub const EXT4_DYNAMIC_INODE_SIZE: u16 = 256;
pub const EXT4_N_BLOCKS: usize = 15;
pub const EXT4_INODE_BLOCK_BYTES: usize = EXT4_N_BLOCKS * 4;
pub const EXT4_EXTENTS_FL: u32 = 0x0008_0000;
pub const EXT4_INDEX_FL: u32 = 0x0000_1000;
pub const EXT4_FEATURE_INCOMPAT_EXTENTS: u32 = 0x0000_0040;
pub const EXT4_FEATURE_INCOMPAT_64BIT: u32 = 0x0000_0080;
pub const EXT4_FEATURE_INCOMPAT_CSUM_SEED: u32 = 0x0002_0000;
pub const EXT4_FEATURE_RO_COMPAT_METADATA_CSUM: u32 = 0x0000_0400;
pub const EXT4_EXTENT_MAGIC: u16 = 0xf30a;
pub const EXT4_DIR_ENTRY_HEADER_LEN: usize = 8;
pub const EXT4_MAX_NAME_LEN: usize = 255;
pub const EXT4_JOURNAL_MAGIC: u32 = 0xc03b_3998;
pub const MAX_METADATA_BLOCK_BYTES: usize = 4096;

const CRC32C_POLY_REVERSED: u32 = 0x82f6_3b78;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ext4Error {
    BufferTooSmall,
    BadMagic,
    ChecksumMismatch,
    InvalidBlockSize,
    InvalidDescriptor,
    InvalidExtent,
    InvalidInode,
    InvalidDirectoryEntry,
    InvalidBitmap,
    JournalReplayNeeded,
    NoSpace,
    Device(DeviceError),
}

impl From<DeviceError> for Ext4Error {
    fn from(value: DeviceError) -> Self {
        Self::Device(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeCursor<'a> {
    bytes: &'a [u8],
}

impl<'a> LeCursor<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    pub fn u8_at(self, offset: usize) -> Result<u8, Ext4Error> {
        self.bytes
            .get(offset)
            .copied()
            .ok_or(Ext4Error::BufferTooSmall)
    }

    pub fn u16_at(self, offset: usize) -> Result<u16, Ext4Error> {
        let bytes = self
            .bytes
            .get(offset..offset.saturating_add(2))
            .ok_or(Ext4Error::BufferTooSmall)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub fn u32_at(self, offset: usize) -> Result<u32, Ext4Error> {
        let bytes = self
            .bytes
            .get(offset..offset.saturating_add(4))
            .ok_or(Ext4Error::BufferTooSmall)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn u64_at(self, offset: usize) -> Result<u64, Ext4Error> {
        let bytes = self
            .bytes
            .get(offset..offset.saturating_add(8))
            .ok_or(Ext4Error::BufferTooSmall)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }
}

fn put_u16(out: &mut [u8], offset: usize, value: u16) -> Result<(), Ext4Error> {
    let dst = out
        .get_mut(offset..offset.saturating_add(2))
        .ok_or(Ext4Error::BufferTooSmall)?;
    dst.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn put_u32(out: &mut [u8], offset: usize, value: u32) -> Result<(), Ext4Error> {
    let dst = out
        .get_mut(offset..offset.saturating_add(4))
        .ok_or(Ext4Error::BufferTooSmall)?;
    dst.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn put_u64(out: &mut [u8], offset: usize, value: u64) -> Result<(), Ext4Error> {
    let dst = out
        .get_mut(offset..offset.saturating_add(8))
        .ok_or(Ext4Error::BufferTooSmall)?;
    dst.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ext4Timestamp {
    pub seconds: u32,
    pub extra_nanos_epoch: u32,
}

impl Ext4Timestamp {
    pub const fn new(seconds: u32, extra_nanos_epoch: u32) -> Self {
        Self {
            seconds,
            extra_nanos_epoch,
        }
    }

    pub const fn unix_seconds(self) -> u64 {
        let epoch_hi = ((self.extra_nanos_epoch >> 30) & 0x3) as u64;
        ((epoch_hi << 32) | self.seconds as u64) & 0x0000_0003_ffff_ffff
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ext4Superblock {
    pub inodes_count: u32,
    pub blocks_count: u64,
    pub reserved_blocks_count: u64,
    pub free_blocks_count: u64,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    pub log_block_size: u32,
    pub log_cluster_size: u32,
    pub blocks_per_group: u32,
    pub clusters_per_group: u32,
    pub inodes_per_group: u32,
    pub mount_time: u32,
    pub write_time: u32,
    pub mount_count: u16,
    pub max_mount_count: u16,
    pub state: u16,
    pub errors: u16,
    pub minor_rev_level: u16,
    pub last_check: u32,
    pub check_interval: u32,
    pub creator_os: u32,
    pub rev_level: u32,
    pub first_inode: u32,
    pub inode_size: u16,
    pub block_group_nr: u16,
    pub feature_compat: u32,
    pub feature_incompat: u32,
    pub feature_ro_compat: u32,
    pub uuid: [u8; 16],
    pub volume_name: [u8; 16],
    pub last_mounted: [u8; 64],
    pub algorithm_usage_bitmap: u32,
    pub descriptor_size: u16,
    pub checksum_seed: u32,
    pub checksum: u32,
}

impl Ext4Superblock {
    pub fn parse(bytes: &[u8]) -> Result<Self, Ext4Error> {
        if bytes.len() < EXT4_SUPERBLOCK_SIZE {
            return Err(Ext4Error::BufferTooSmall);
        }
        let cur = LeCursor::new(bytes);
        if cur.u16_at(0x38)? != EXT4_SUPER_MAGIC {
            return Err(Ext4Error::BadMagic);
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&bytes[0x68..0x78]);
        let mut volume_name = [0u8; 16];
        volume_name.copy_from_slice(&bytes[0x78..0x88]);
        let mut last_mounted = [0u8; 64];
        last_mounted.copy_from_slice(&bytes[0x88..0xc8]);
        let blocks_lo = cur.u32_at(0x04)? as u64;
        let reserved_lo = cur.u32_at(0x08)? as u64;
        let free_lo = cur.u32_at(0x0c)? as u64;
        let blocks_hi = cur.u32_at(0x150)? as u64;
        let reserved_hi = cur.u32_at(0x154)? as u64;
        let free_hi = cur.u32_at(0x158)? as u64;
        let descriptor_size = cur.u16_at(0xfe)?;
        Ok(Self {
            inodes_count: cur.u32_at(0x00)?,
            blocks_count: blocks_lo | (blocks_hi << 32),
            reserved_blocks_count: reserved_lo | (reserved_hi << 32),
            free_blocks_count: free_lo | (free_hi << 32),
            free_inodes_count: cur.u32_at(0x10)?,
            first_data_block: cur.u32_at(0x14)?,
            log_block_size: cur.u32_at(0x18)?,
            log_cluster_size: cur.u32_at(0x1c)?,
            blocks_per_group: cur.u32_at(0x20)?,
            clusters_per_group: cur.u32_at(0x24)?,
            inodes_per_group: cur.u32_at(0x28)?,
            mount_time: cur.u32_at(0x2c)?,
            write_time: cur.u32_at(0x30)?,
            mount_count: cur.u16_at(0x34)?,
            max_mount_count: cur.u16_at(0x36)?,
            state: cur.u16_at(0x3a)?,
            errors: cur.u16_at(0x3c)?,
            minor_rev_level: cur.u16_at(0x3e)?,
            last_check: cur.u32_at(0x40)?,
            check_interval: cur.u32_at(0x44)?,
            creator_os: cur.u32_at(0x48)?,
            rev_level: cur.u32_at(0x4c)?,
            first_inode: cur.u32_at(0x54)?,
            inode_size: cur.u16_at(0x58)?,
            block_group_nr: cur.u16_at(0x5a)?,
            feature_compat: cur.u32_at(0x5c)?,
            feature_incompat: cur.u32_at(0x60)?,
            feature_ro_compat: cur.u32_at(0x64)?,
            uuid,
            volume_name,
            last_mounted,
            algorithm_usage_bitmap: cur.u32_at(0xc8)?,
            descriptor_size,
            checksum_seed: cur.u32_at(0x270)?,
            checksum: cur.u32_at(0x3fc)?,
        })
    }

    pub fn serialize(&self, out: &mut [u8]) -> Result<(), Ext4Error> {
        if out.len() < EXT4_SUPERBLOCK_SIZE {
            return Err(Ext4Error::BufferTooSmall);
        }
        out[..EXT4_SUPERBLOCK_SIZE].fill(0);
        put_u32(out, 0x00, self.inodes_count)?;
        put_u32(out, 0x04, self.blocks_count as u32)?;
        put_u32(out, 0x08, self.reserved_blocks_count as u32)?;
        put_u32(out, 0x0c, self.free_blocks_count as u32)?;
        put_u32(out, 0x10, self.free_inodes_count)?;
        put_u32(out, 0x14, self.first_data_block)?;
        put_u32(out, 0x18, self.log_block_size)?;
        put_u32(out, 0x1c, self.log_cluster_size)?;
        put_u32(out, 0x20, self.blocks_per_group)?;
        put_u32(out, 0x24, self.clusters_per_group)?;
        put_u32(out, 0x28, self.inodes_per_group)?;
        put_u32(out, 0x2c, self.mount_time)?;
        put_u32(out, 0x30, self.write_time)?;
        put_u16(out, 0x34, self.mount_count)?;
        put_u16(out, 0x36, self.max_mount_count)?;
        put_u16(out, 0x38, EXT4_SUPER_MAGIC)?;
        put_u16(out, 0x3a, self.state)?;
        put_u16(out, 0x3c, self.errors)?;
        put_u16(out, 0x3e, self.minor_rev_level)?;
        put_u32(out, 0x40, self.last_check)?;
        put_u32(out, 0x44, self.check_interval)?;
        put_u32(out, 0x48, self.creator_os)?;
        put_u32(out, 0x4c, self.rev_level)?;
        put_u32(out, 0x54, self.first_inode)?;
        put_u16(out, 0x58, self.inode_size)?;
        put_u16(out, 0x5a, self.block_group_nr)?;
        put_u32(out, 0x5c, self.feature_compat)?;
        put_u32(out, 0x60, self.feature_incompat)?;
        put_u32(out, 0x64, self.feature_ro_compat)?;
        out[0x68..0x78].copy_from_slice(&self.uuid);
        out[0x78..0x88].copy_from_slice(&self.volume_name);
        out[0x88..0xc8].copy_from_slice(&self.last_mounted);
        put_u32(out, 0xc8, self.algorithm_usage_bitmap)?;
        put_u16(out, 0xfe, self.descriptor_size)?;
        put_u32(out, 0x150, (self.blocks_count >> 32) as u32)?;
        put_u32(out, 0x154, (self.reserved_blocks_count >> 32) as u32)?;
        put_u32(out, 0x158, (self.free_blocks_count >> 32) as u32)?;
        put_u32(out, 0x270, self.checksum_seed)?;
        put_u32(out, 0x3fc, self.checksum)?;
        Ok(())
    }

    pub const fn block_size(&self) -> Result<u32, Ext4Error> {
        if self.log_block_size > 16 {
            return Err(Ext4Error::InvalidBlockSize);
        }
        Ok(1024u32 << self.log_block_size)
    }

    pub const fn metadata_checksums_enabled(&self) -> bool {
        (self.feature_ro_compat & EXT4_FEATURE_RO_COMPAT_METADATA_CSUM) != 0
    }

    pub const fn extents_enabled(&self) -> bool {
        (self.feature_incompat & EXT4_FEATURE_INCOMPAT_EXTENTS) != 0
    }

    pub fn to_vfs_superblock(&self) -> VfsSuperBlock {
        VfsSuperBlock {
            device: 0,
            block_size: self.block_size().unwrap_or(0),
            total_blocks: self.blocks_count,
            free_blocks: self.free_blocks_count,
            root: InodeId::ROOT,
            read_only: false,
        }
    }

    pub fn checksum_without_tail(&self, serialized: &[u8]) -> Result<u32, Ext4Error> {
        if serialized.len() < EXT4_SUPERBLOCK_SIZE {
            return Err(Ext4Error::BufferTooSmall);
        }
        let limit = EXT4_SUPERBLOCK_SIZE - 4;
        Ok(crc32c(self.checksum_seed, &serialized[..limit]))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockGroupDescriptor {
    pub block_bitmap: u64,
    pub inode_bitmap: u64,
    pub inode_table: u64,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub used_dirs_count: u32,
    pub flags: u16,
    pub exclude_bitmap: u64,
    pub block_bitmap_checksum: u32,
    pub inode_bitmap_checksum: u32,
    pub itable_unused: u32,
    pub checksum: u16,
}

impl BlockGroupDescriptor {
    pub const MIN_SIZE: usize = 32;
    pub const FULL_SIZE: usize = 64;

    pub fn parse(bytes: &[u8]) -> Result<Self, Ext4Error> {
        if bytes.len() < Self::MIN_SIZE {
            return Err(Ext4Error::BufferTooSmall);
        }
        let cur = LeCursor::new(bytes);
        let hi = bytes.len() >= Self::FULL_SIZE;
        let high_u32 = |offset: usize| -> Result<u32, Ext4Error> {
            if hi {
                cur.u32_at(offset)
            } else {
                Ok(0)
            }
        };
        let high_u16 = |offset: usize| -> Result<u32, Ext4Error> {
            if hi {
                Ok(cur.u16_at(offset)? as u32)
            } else {
                Ok(0)
            }
        };
        Ok(Self {
            block_bitmap: cur.u32_at(0x00)? as u64 | ((high_u32(0x20)? as u64) << 32),
            inode_bitmap: cur.u32_at(0x04)? as u64 | ((high_u32(0x24)? as u64) << 32),
            inode_table: cur.u32_at(0x08)? as u64 | ((high_u32(0x28)? as u64) << 32),
            free_blocks_count: cur.u16_at(0x0c)? as u32 | (high_u16(0x2c)? << 16),
            free_inodes_count: cur.u16_at(0x0e)? as u32 | (high_u16(0x2e)? << 16),
            used_dirs_count: cur.u16_at(0x10)? as u32 | (high_u16(0x30)? << 16),
            flags: cur.u16_at(0x12)?,
            exclude_bitmap: cur.u32_at(0x14)? as u64 | ((high_u32(0x34)? as u64) << 32),
            block_bitmap_checksum: cur.u16_at(0x18)? as u32 | (high_u16(0x38)? << 16),
            inode_bitmap_checksum: cur.u16_at(0x1a)? as u32 | (high_u16(0x3a)? << 16),
            itable_unused: cur.u16_at(0x1c)? as u32 | (high_u16(0x3c)? << 16),
            checksum: cur.u16_at(0x1e)?,
        })
    }

    pub fn serialize(&self, out: &mut [u8]) -> Result<usize, Ext4Error> {
        let size = if out.len() >= Self::FULL_SIZE {
            Self::FULL_SIZE
        } else if out.len() >= Self::MIN_SIZE {
            Self::MIN_SIZE
        } else {
            return Err(Ext4Error::BufferTooSmall);
        };
        out[..size].fill(0);
        put_u32(out, 0x00, self.block_bitmap as u32)?;
        put_u32(out, 0x04, self.inode_bitmap as u32)?;
        put_u32(out, 0x08, self.inode_table as u32)?;
        put_u16(out, 0x0c, self.free_blocks_count as u16)?;
        put_u16(out, 0x0e, self.free_inodes_count as u16)?;
        put_u16(out, 0x10, self.used_dirs_count as u16)?;
        put_u16(out, 0x12, self.flags)?;
        put_u32(out, 0x14, self.exclude_bitmap as u32)?;
        put_u16(out, 0x18, self.block_bitmap_checksum as u16)?;
        put_u16(out, 0x1a, self.inode_bitmap_checksum as u16)?;
        put_u16(out, 0x1c, self.itable_unused as u16)?;
        put_u16(out, 0x1e, self.checksum)?;
        if size == Self::FULL_SIZE {
            put_u32(out, 0x20, (self.block_bitmap >> 32) as u32)?;
            put_u32(out, 0x24, (self.inode_bitmap >> 32) as u32)?;
            put_u32(out, 0x28, (self.inode_table >> 32) as u32)?;
            put_u16(out, 0x2c, (self.free_blocks_count >> 16) as u16)?;
            put_u16(out, 0x2e, (self.free_inodes_count >> 16) as u16)?;
            put_u16(out, 0x30, (self.used_dirs_count >> 16) as u16)?;
            put_u32(out, 0x34, (self.exclude_bitmap >> 32) as u32)?;
            put_u16(out, 0x38, (self.block_bitmap_checksum >> 16) as u16)?;
            put_u16(out, 0x3a, (self.inode_bitmap_checksum >> 16) as u16)?;
            put_u16(out, 0x3c, (self.itable_unused >> 16) as u16)?;
        }
        Ok(size)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtentHeader {
    pub entries: u16,
    pub max_entries: u16,
    pub depth: u16,
    pub generation: u32,
}

impl ExtentHeader {
    pub const SIZE: usize = 12;

    pub fn parse(bytes: &[u8]) -> Result<Self, Ext4Error> {
        let cur = LeCursor::new(bytes);
        if cur.u16_at(0)? != EXT4_EXTENT_MAGIC {
            return Err(Ext4Error::BadMagic);
        }
        Ok(Self {
            entries: cur.u16_at(2)?,
            max_entries: cur.u16_at(4)?,
            depth: cur.u16_at(6)?,
            generation: cur.u32_at(8)?,
        })
    }

    pub fn serialize(&self, out: &mut [u8]) -> Result<(), Ext4Error> {
        if self.entries > self.max_entries {
            return Err(Ext4Error::InvalidExtent);
        }
        put_u16(out, 0, EXT4_EXTENT_MAGIC)?;
        put_u16(out, 2, self.entries)?;
        put_u16(out, 4, self.max_entries)?;
        put_u16(out, 6, self.depth)?;
        put_u32(out, 8, self.generation)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub logical_block: u32,
    pub len: u16,
    pub start: u64,
}

impl Extent {
    pub const SIZE: usize = 12;

    pub fn parse(bytes: &[u8]) -> Result<Self, Ext4Error> {
        let cur = LeCursor::new(bytes);
        Ok(Self {
            logical_block: cur.u32_at(0)?,
            len: cur.u16_at(4)?,
            start: cur.u32_at(8)? as u64 | ((cur.u16_at(6)? as u64) << 32),
        })
    }

    pub fn serialize(&self, out: &mut [u8]) -> Result<(), Ext4Error> {
        put_u32(out, 0, self.logical_block)?;
        put_u16(out, 4, self.len)?;
        put_u16(out, 6, (self.start >> 32) as u16)?;
        put_u32(out, 8, self.start as u32)?;
        Ok(())
    }

    pub const fn end_logical_block(self) -> u32 {
        self.logical_block.saturating_add(self.len as u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtentIndex {
    pub logical_block: u32,
    pub leaf: u64,
}

impl ExtentIndex {
    pub const SIZE: usize = 12;

    pub fn parse(bytes: &[u8]) -> Result<Self, Ext4Error> {
        let cur = LeCursor::new(bytes);
        Ok(Self {
            logical_block: cur.u32_at(0)?,
            leaf: cur.u32_at(4)? as u64 | ((cur.u16_at(8)? as u64) << 32),
        })
    }

    pub fn serialize(&self, out: &mut [u8]) -> Result<(), Ext4Error> {
        put_u32(out, 0, self.logical_block)?;
        put_u32(out, 4, self.leaf as u32)?;
        put_u16(out, 8, (self.leaf >> 32) as u16)?;
        put_u16(out, 10, 0)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtentTree<const MAX_EXTENTS: usize> {
    pub header: ExtentHeader,
    pub extents: [Option<Extent>; MAX_EXTENTS],
}

impl<const MAX_EXTENTS: usize> ExtentTree<MAX_EXTENTS> {
    pub const fn empty() -> Self {
        Self {
            header: ExtentHeader {
                entries: 0,
                max_entries: MAX_EXTENTS as u16,
                depth: 0,
                generation: 0,
            },
            extents: [None; MAX_EXTENTS],
        }
    }

    pub fn parse_leaf(bytes: &[u8]) -> Result<Self, Ext4Error> {
        let header = ExtentHeader::parse(bytes)?;
        if header.depth != 0 || header.entries as usize > MAX_EXTENTS {
            return Err(Ext4Error::InvalidExtent);
        }
        let mut tree = Self::empty();
        tree.header = header;
        let mut idx = 0usize;
        while idx < header.entries as usize {
            let off = ExtentHeader::SIZE + idx * Extent::SIZE;
            tree.extents[idx] = Some(Extent::parse(
                bytes
                    .get(off..off + Extent::SIZE)
                    .ok_or(Ext4Error::BufferTooSmall)?,
            )?);
            idx += 1;
        }
        Ok(tree)
    }

    pub fn append(&mut self, extent: Extent) -> Result<(), Ext4Error> {
        if self.header.entries as usize >= MAX_EXTENTS {
            return Err(Ext4Error::NoSpace);
        }
        let idx = self.header.entries as usize;
        self.extents[idx] = Some(extent);
        self.header.entries += 1;
        Ok(())
    }

    pub fn serialize_leaf(&self, out: &mut [u8]) -> Result<usize, Ext4Error> {
        let needed = ExtentHeader::SIZE + self.header.entries as usize * Extent::SIZE;
        if out.len() < needed {
            return Err(Ext4Error::BufferTooSmall);
        }
        self.header.serialize(out)?;
        let mut idx = 0usize;
        while idx < self.header.entries as usize {
            let off = ExtentHeader::SIZE + idx * Extent::SIZE;
            self.extents[idx]
                .ok_or(Ext4Error::InvalidExtent)?
                .serialize(&mut out[off..off + Extent::SIZE])?;
            idx += 1;
        }
        Ok(needed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InodeMode(u16);

impl InodeMode {
    pub const FIFO: Self = Self(0o010000);
    pub const CHAR_DEVICE: Self = Self(0o020000);
    pub const DIRECTORY: Self = Self(0o040000);
    pub const BLOCK_DEVICE: Self = Self(0o060000);
    pub const REGULAR: Self = Self(0o100000);
    pub const SYMLINK: Self = Self(0o120000);
    pub const SOCKET: Self = Self(0o140000);

    pub const fn new(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn permissions(self) -> Permissions {
        Permissions::new(self.0 & 0o777, 0, 0)
    }

    pub const fn kind(self) -> InodeKind {
        match self.0 & 0o170000 {
            0o040000 => InodeKind::Directory,
            0o120000 => InodeKind::Symlink,
            0o060000 => InodeKind::BlockDevice,
            0o020000 => InodeKind::CharDevice,
            0o010000 => InodeKind::Fifo,
            0o140000 => InodeKind::Socket,
            _ => InodeKind::RegularFile,
        }
    }

    pub const fn from_kind(kind: InodeKind, perms: u16) -> Self {
        let file_type = match kind {
            InodeKind::Directory => Self::DIRECTORY.0,
            InodeKind::Symlink => Self::SYMLINK.0,
            InodeKind::BlockDevice => Self::BLOCK_DEVICE.0,
            InodeKind::CharDevice => Self::CHAR_DEVICE.0,
            InodeKind::Fifo => Self::FIFO.0,
            InodeKind::Socket => Self::SOCKET.0,
            InodeKind::RegularFile => Self::REGULAR.0,
        };
        Self(file_type | (perms & 0o7777))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InodeRecord {
    pub mode: InodeMode,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub access_time: Ext4Timestamp,
    pub change_time: Ext4Timestamp,
    pub modification_time: Ext4Timestamp,
    pub deletion_time: u32,
    pub links_count: u16,
    pub blocks_count: u64,
    pub flags: u32,
    pub block: [u8; EXT4_INODE_BLOCK_BYTES],
    pub generation: u32,
    pub file_acl: u64,
    pub checksum: u32,
}

impl InodeRecord {
    pub fn parse(bytes: &[u8]) -> Result<Self, Ext4Error> {
        if bytes.len() < EXT4_GOOD_OLD_INODE_SIZE as usize {
            return Err(Ext4Error::BufferTooSmall);
        }
        let cur = LeCursor::new(bytes);
        let mut block = [0u8; EXT4_INODE_BLOCK_BYTES];
        block.copy_from_slice(&bytes[0x28..0x64]);
        let has_extra = bytes.len() >= EXT4_DYNAMIC_INODE_SIZE as usize;
        let uid = cur.u16_at(0x02)? as u32 | ((cur.u16_at(0x78).unwrap_or(0) as u32) << 16);
        let gid = cur.u16_at(0x18)? as u32 | ((cur.u16_at(0x7a).unwrap_or(0) as u32) << 16);
        let size = cur.u32_at(0x04)? as u64 | ((cur.u32_at(0x6c).unwrap_or(0) as u64) << 32);
        let blocks = cur.u32_at(0x1c)? as u64 | ((cur.u16_at(0x74).unwrap_or(0) as u64) << 32);
        Ok(Self {
            mode: InodeMode::new(cur.u16_at(0x00)?),
            uid,
            gid,
            size,
            access_time: Ext4Timestamp::new(cur.u32_at(0x08)?, cur.u32_at(0x88).unwrap_or(0)),
            change_time: Ext4Timestamp::new(cur.u32_at(0x0c)?, cur.u32_at(0x8c).unwrap_or(0)),
            modification_time: Ext4Timestamp::new(cur.u32_at(0x10)?, cur.u32_at(0x90).unwrap_or(0)),
            deletion_time: cur.u32_at(0x14)?,
            links_count: cur.u16_at(0x1a)?,
            blocks_count: blocks,
            flags: cur.u32_at(0x20)?,
            block,
            generation: cur.u32_at(0x64)?,
            file_acl: cur.u32_at(0x68)? as u64 | ((cur.u16_at(0x76).unwrap_or(0) as u64) << 32),
            checksum: if has_extra {
                cur.u16_at(0x7c)? as u32 | ((cur.u16_at(0x82)? as u32) << 16)
            } else {
                0
            },
        })
    }

    pub fn serialize(&self, out: &mut [u8]) -> Result<usize, Ext4Error> {
        if out.len() < EXT4_GOOD_OLD_INODE_SIZE as usize {
            return Err(Ext4Error::BufferTooSmall);
        }
        let size = min(out.len(), EXT4_DYNAMIC_INODE_SIZE as usize);
        out[..size].fill(0);
        put_u16(out, 0x00, self.mode.bits())?;
        put_u16(out, 0x02, self.uid as u16)?;
        put_u32(out, 0x04, self.size as u32)?;
        put_u32(out, 0x08, self.access_time.seconds)?;
        put_u32(out, 0x0c, self.change_time.seconds)?;
        put_u32(out, 0x10, self.modification_time.seconds)?;
        put_u32(out, 0x14, self.deletion_time)?;
        put_u16(out, 0x18, self.gid as u16)?;
        put_u16(out, 0x1a, self.links_count)?;
        put_u32(out, 0x1c, self.blocks_count as u32)?;
        put_u32(out, 0x20, self.flags)?;
        out[0x28..0x64].copy_from_slice(&self.block);
        put_u32(out, 0x64, self.generation)?;
        put_u32(out, 0x68, self.file_acl as u32)?;
        put_u32(out, 0x6c, (self.size >> 32) as u32)?;
        if size >= EXT4_DYNAMIC_INODE_SIZE as usize {
            put_u16(out, 0x74, (self.blocks_count >> 32) as u16)?;
            put_u16(out, 0x76, (self.file_acl >> 32) as u16)?;
            put_u16(out, 0x78, (self.uid >> 16) as u16)?;
            put_u16(out, 0x7a, (self.gid >> 16) as u16)?;
            put_u16(out, 0x7c, self.checksum as u16)?;
            put_u16(out, 0x80, 32)?;
            put_u16(out, 0x82, (self.checksum >> 16) as u16)?;
            put_u32(out, 0x88, self.access_time.extra_nanos_epoch)?;
            put_u32(out, 0x8c, self.change_time.extra_nanos_epoch)?;
            put_u32(out, 0x90, self.modification_time.extra_nanos_epoch)?;
        }
        Ok(size)
    }

    pub const fn uses_extents(&self) -> bool {
        (self.flags & EXT4_EXTENTS_FL) != 0
    }

    pub fn metadata(&self, inode: InodeId) -> InodeMetadata {
        InodeMetadata::with_links(
            inode,
            self.mode.kind(),
            self.size,
            Permissions::new(self.mode.bits() & 0o777, self.uid as u16, self.gid as u16),
            self.links_count,
        )
    }

    pub fn inline_symlink(&self) -> Option<&[u8]> {
        if self.mode.kind() == InodeKind::Symlink && self.size <= EXT4_INODE_BLOCK_BYTES as u64 {
            Some(&self.block[..self.size as usize])
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryFileType {
    Unknown = 0,
    Regular = 1,
    Directory = 2,
    CharDevice = 3,
    BlockDevice = 4,
    Fifo = 5,
    Socket = 6,
    Symlink = 7,
}

impl DirectoryFileType {
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Regular,
            2 => Self::Directory,
            3 => Self::CharDevice,
            4 => Self::BlockDevice,
            5 => Self::Fifo,
            6 => Self::Socket,
            7 => Self::Symlink,
            _ => Self::Unknown,
        }
    }

    pub const fn from_kind(kind: InodeKind) -> Self {
        match kind {
            InodeKind::Directory => Self::Directory,
            InodeKind::Symlink => Self::Symlink,
            InodeKind::BlockDevice => Self::BlockDevice,
            InodeKind::CharDevice => Self::CharDevice,
            InodeKind::Fifo => Self::Fifo,
            InodeKind::Socket => Self::Socket,
            InodeKind::RegularFile => Self::Regular,
        }
    }

    pub const fn kind(self) -> InodeKind {
        match self {
            Self::Directory => InodeKind::Directory,
            Self::Symlink => InodeKind::Symlink,
            Self::BlockDevice => InodeKind::BlockDevice,
            Self::CharDevice => InodeKind::CharDevice,
            Self::Fifo => InodeKind::Fifo,
            Self::Socket => InodeKind::Socket,
            Self::Unknown | Self::Regular => InodeKind::RegularFile,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryEntry<'a> {
    pub inode: u32,
    pub record_len: u16,
    pub file_type: DirectoryFileType,
    pub name: &'a [u8],
}

impl<'a> DirectoryEntry<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Ext4Error> {
        if bytes.len() < EXT4_DIR_ENTRY_HEADER_LEN {
            return Err(Ext4Error::BufferTooSmall);
        }
        let cur = LeCursor::new(bytes);
        let record_len = cur.u16_at(4)?;
        let name_len = cur.u8_at(6)? as usize;
        if (record_len as usize) > bytes.len()
            || (record_len as usize) < EXT4_DIR_ENTRY_HEADER_LEN
            || name_len > EXT4_MAX_NAME_LEN
            || EXT4_DIR_ENTRY_HEADER_LEN + name_len > record_len as usize
        {
            return Err(Ext4Error::InvalidDirectoryEntry);
        }
        Ok(Self {
            inode: cur.u32_at(0)?,
            record_len,
            file_type: DirectoryFileType::from_u8(cur.u8_at(7)?),
            name: &bytes[EXT4_DIR_ENTRY_HEADER_LEN..EXT4_DIR_ENTRY_HEADER_LEN + name_len],
        })
    }

    pub fn serialize_into(
        inode: u32,
        file_type: DirectoryFileType,
        name: &[u8],
        out: &mut [u8],
    ) -> Result<usize, Ext4Error> {
        if name.len() > EXT4_MAX_NAME_LEN {
            return Err(Ext4Error::InvalidDirectoryEntry);
        }
        let record_len = align4(EXT4_DIR_ENTRY_HEADER_LEN + name.len());
        if out.len() < record_len {
            return Err(Ext4Error::BufferTooSmall);
        }
        out[..record_len].fill(0);
        put_u32(out, 0, inode)?;
        put_u16(out, 4, record_len as u16)?;
        out[6] = name.len() as u8;
        out[7] = file_type as u8;
        out[EXT4_DIR_ENTRY_HEADER_LEN..EXT4_DIR_ENTRY_HEADER_LEN + name.len()]
            .copy_from_slice(name);
        Ok(record_len)
    }
}

const fn align4(value: usize) -> usize {
    (value + 3) & !3
}

pub struct Bitmap<'a> {
    bytes: &'a [u8],
}

impl<'a> Bitmap<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    pub fn is_set(&self, bit: usize) -> Result<bool, Ext4Error> {
        let byte = *self.bytes.get(bit / 8).ok_or(Ext4Error::InvalidBitmap)?;
        Ok((byte & (1 << (bit % 8))) != 0)
    }

    pub fn first_zero_from(&self, start_bit: usize, total_bits: usize) -> Option<usize> {
        let mut bit = start_bit;
        while bit < total_bits {
            if self.is_set(bit).ok() == Some(false) {
                return Some(bit);
            }
            bit += 1;
        }
        None
    }
}

pub struct BitmapMut<'a> {
    bytes: &'a mut [u8],
}

impl<'a> BitmapMut<'a> {
    pub fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes }
    }

    pub fn set(&mut self, bit: usize, value: bool) -> Result<(), Ext4Error> {
        let byte = self
            .bytes
            .get_mut(bit / 8)
            .ok_or(Ext4Error::InvalidBitmap)?;
        let mask = 1u8 << (bit % 8);
        if value {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
        Ok(())
    }

    pub fn as_bitmap(&self) -> Bitmap<'_> {
        Bitmap::new(self.bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChecksumContext {
    pub seed: u32,
    pub filesystem_uuid: [u8; 16],
}

impl ChecksumContext {
    pub fn metadata_crc(&self, inode: u32, generation: u32, payload: &[u8]) -> u32 {
        let mut crc = crc32c(self.seed, &self.filesystem_uuid);
        crc = crc32c_extend(crc, &inode.to_le_bytes());
        crc = crc32c_extend(crc, &generation.to_le_bytes());
        crc32c_extend(crc, payload)
    }

    pub fn bitmap_crc(&self, group: u32, bitmap: &[u8]) -> u32 {
        let crc = crc32c(self.seed, &self.filesystem_uuid);
        let crc = crc32c_extend(crc, &group.to_le_bytes());
        crc32c_extend(crc, bitmap)
    }
}

pub fn crc32c(seed: u32, bytes: &[u8]) -> u32 {
    crc32c_extend(!seed, bytes) ^ 0xffff_ffff
}

pub fn crc32c_extend(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc ^= *byte as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (CRC32C_POLY_REVERSED & mask);
            bit += 1;
        }
    }
    crc
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalRecordKind {
    Descriptor,
    Metadata,
    Commit,
    Revoke,
    Superblock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalRecord {
    pub sequence: u32,
    pub block: u64,
    pub len: u16,
    pub checksum: u32,
    pub kind: JournalRecordKind,
}

impl JournalRecord {
    pub const SIZE: usize = 32;

    pub fn parse(bytes: &[u8]) -> Result<Self, Ext4Error> {
        let cur = LeCursor::new(bytes);
        if cur.u32_at(0)? != EXT4_JOURNAL_MAGIC {
            return Err(Ext4Error::BadMagic);
        }
        let kind = match cur.u32_at(4)? {
            1 => JournalRecordKind::Descriptor,
            2 => JournalRecordKind::Commit,
            3 => JournalRecordKind::Superblock,
            5 => JournalRecordKind::Revoke,
            _ => JournalRecordKind::Metadata,
        };
        Ok(Self {
            kind,
            sequence: cur.u32_at(8)?,
            block: cur.u64_at(12)?,
            len: cur.u16_at(20)?,
            checksum: cur.u32_at(24)?,
        })
    }

    pub fn serialize(&self, out: &mut [u8]) -> Result<(), Ext4Error> {
        put_u32(out, 0, EXT4_JOURNAL_MAGIC)?;
        put_u32(
            out,
            4,
            match self.kind {
                JournalRecordKind::Descriptor => 1,
                JournalRecordKind::Commit => 2,
                JournalRecordKind::Superblock => 3,
                JournalRecordKind::Revoke => 5,
                JournalRecordKind::Metadata => 6,
            },
        )?;
        put_u32(out, 8, self.sequence)?;
        put_u64(out, 12, self.block)?;
        put_u16(out, 20, self.len)?;
        put_u32(out, 24, self.checksum)?;
        put_u32(out, 28, 0)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CowMetadata {
    pub active_generation: u64,
    pub shadow_generation: u64,
    pub active_root: u64,
    pub shadow_root: u64,
    pub flags: u32,
    pub checksum: u32,
}

impl CowMetadata {
    pub const SIZE: usize = 40;

    pub fn parse(bytes: &[u8]) -> Result<Self, Ext4Error> {
        let cur = LeCursor::new(bytes);
        Ok(Self {
            active_generation: cur.u64_at(0)?,
            shadow_generation: cur.u64_at(8)?,
            active_root: cur.u64_at(16)?,
            shadow_root: cur.u64_at(24)?,
            flags: cur.u32_at(32)?,
            checksum: cur.u32_at(36)?,
        })
    }

    pub fn serialize(&self, out: &mut [u8]) -> Result<(), Ext4Error> {
        put_u64(out, 0, self.active_generation)?;
        put_u64(out, 8, self.shadow_generation)?;
        put_u64(out, 16, self.active_root)?;
        put_u64(out, 24, self.shadow_root)?;
        put_u32(out, 32, self.flags)?;
        put_u32(out, 36, self.checksum)?;
        Ok(())
    }

    pub fn promote_shadow(&mut self) {
        self.active_generation = self.shadow_generation;
        self.active_root = self.shadow_root;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SsdUsbOptions {
    pub erase_block_sectors: u32,
    pub max_batched_metadata: u16,
    pub delayed_allocation: bool,
    pub issue_discard: bool,
    pub explicit_flush_barriers: bool,
}

impl SsdUsbOptions {
    pub const fn flash_friendly(erase_block_sectors: u32) -> Self {
        Self {
            erase_block_sectors,
            max_batched_metadata: 32,
            delayed_allocation: true,
            issue_discard: true,
            explicit_flush_barriers: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocationGoal {
    pub logical_block: u32,
    pub blocks: u32,
    pub preferred_group: u32,
    pub erase_block_blocks: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtentAllocationPlan {
    pub group: u32,
    pub first_block: u64,
    pub len: u32,
    pub aligned_to_erase_block: bool,
    pub delayed: bool,
}

impl ExtentAllocationPlan {
    pub fn extent(self, logical_block: u32) -> Extent {
        Extent {
            logical_block,
            len: min(self.len, u16::MAX as u32) as u16,
            start: self.first_block,
        }
    }
}

pub trait DelayedAllocationHook {
    fn reserve(&self, goal: AllocationGoal) -> Result<ExtentAllocationPlan, Ext4Error>;
    fn materialize(&self, plan: ExtentAllocationPlan) -> Result<Extent, Ext4Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetadataBlock {
    pub block: u64,
    pub len: usize,
    pub data: [u8; MAX_METADATA_BLOCK_BYTES],
}

impl MetadataBlock {
    pub const fn empty() -> Self {
        Self {
            block: 0,
            len: 0,
            data: [0; MAX_METADATA_BLOCK_BYTES],
        }
    }

    pub fn new(block: u64, bytes: &[u8]) -> Result<Self, Ext4Error> {
        if bytes.len() > MAX_METADATA_BLOCK_BYTES {
            return Err(Ext4Error::BufferTooSmall);
        }
        let mut out = Self::empty();
        out.block = block;
        out.len = bytes.len();
        out.data[..bytes.len()].copy_from_slice(bytes);
        Ok(out)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetadataCommitBatch<const MAX: usize> {
    entries: [Option<MetadataBlock>; MAX],
    len: usize,
    pub sequence: u32,
}

impl<const MAX: usize> MetadataCommitBatch<MAX> {
    pub const fn new(sequence: u32) -> Self {
        Self {
            entries: [None; MAX],
            len: 0,
            sequence,
        }
    }

    pub fn push(&mut self, block: MetadataBlock) -> Result<(), Ext4Error> {
        if self.len >= MAX {
            return Err(Ext4Error::NoSpace);
        }
        self.entries[self.len] = Some(block);
        self.len += 1;
        Ok(())
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entry(&self, index: usize) -> Option<MetadataBlock> {
        if index < self.len {
            self.entries[index]
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountRecovery {
    Clean,
    ReplayedJournal { records: usize },
    PromotedCowShadow { generation: u64 },
}

pub struct Ext4Backend<'a> {
    device: &'a dyn BlockStorageDevice,
    pub superblock: Ext4Superblock,
    pub options: SsdUsbOptions,
}

impl<'a> Ext4Backend<'a> {
    pub fn mount(
        device: &'a dyn BlockStorageDevice,
        superblock_bytes: &[u8],
        options: SsdUsbOptions,
    ) -> Result<Self, Ext4Error> {
        let superblock = Ext4Superblock::parse(superblock_bytes)?;
        let block_size = superblock.block_size()? as usize;
        if block_size == 0 || block_size % device.sector_size() != 0 {
            return Err(Ext4Error::InvalidBlockSize);
        }
        Ok(Self {
            device,
            superblock,
            options,
        })
    }

    pub fn plan_extent_first_allocation(&self, goal: AllocationGoal) -> ExtentAllocationPlan {
        let blocks_per_group = self.superblock.blocks_per_group.max(1) as u64;
        let erase_block_blocks = goal.erase_block_blocks.max(1) as u64;
        let group = goal.preferred_group;
        let group_start = group as u64 * blocks_per_group;
        let logical_bias = goal.logical_block as u64 % blocks_per_group;
        let unaligned = group_start + logical_bias;
        let aligned = align_up_u64(unaligned, erase_block_blocks);
        let room = group_start
            .saturating_add(blocks_per_group)
            .saturating_sub(aligned);
        ExtentAllocationPlan {
            group,
            first_block: if room == 0 { group_start } else { aligned },
            len: min(goal.blocks as u64, room.max(1)) as u32,
            aligned_to_erase_block: room != 0 && aligned % erase_block_blocks == 0,
            delayed: self.options.delayed_allocation,
        }
    }

    pub fn discard_extent(&self, extent: Extent) -> Result<(), Ext4Error> {
        if !self.options.issue_discard || extent.len == 0 {
            return Ok(());
        }
        let sectors_per_block =
            self.superblock.block_size()? as u64 / self.device.sector_size() as u64;
        self.device.discard(
            extent.start.saturating_mul(sectors_per_block),
            extent.len as u64 * sectors_per_block,
        )?;
        Ok(())
    }

    pub fn flush_barrier(&self) -> Result<(), Ext4Error> {
        if self.options.explicit_flush_barriers {
            self.device.flush()?;
        }
        Ok(())
    }

    pub fn commit_metadata_batch<const MAX: usize>(
        &self,
        batch: &MetadataCommitBatch<MAX>,
    ) -> Result<(), Ext4Error> {
        if batch.is_empty() {
            return Ok(());
        }
        let sectors_per_block =
            self.superblock.block_size()? as u64 / self.device.sector_size() as u64;
        self.flush_barrier()?;
        let mut idx = 0usize;
        while idx < batch.len() {
            let entry = batch.entry(idx).ok_or(Ext4Error::InvalidDescriptor)?;
            if entry.len == 0 || entry.len % self.device.sector_size() != 0 {
                return Err(Ext4Error::InvalidDescriptor);
            }
            self.device.write_sectors(
                entry.block.saturating_mul(sectors_per_block),
                &entry.data[..entry.len],
            )?;
            idx += 1;
        }
        self.flush_barrier()
    }

    pub fn recover_mount(
        &self,
        journal: &[JournalRecord],
        cow: Option<&mut CowMetadata>,
    ) -> Result<MountRecovery, Ext4Error> {
        let mut replayed = 0usize;
        let mut idx = 0usize;
        while idx < journal.len() {
            if journal[idx].kind == JournalRecordKind::Commit {
                replayed += 1;
            }
            idx += 1;
        }
        if replayed != 0 {
            self.flush_barrier()?;
            return Ok(MountRecovery::ReplayedJournal { records: replayed });
        }
        if let Some(cow) = cow {
            if cow.shadow_generation > cow.active_generation {
                cow.promote_shadow();
                self.flush_barrier()?;
                return Ok(MountRecovery::PromotedCowShadow {
                    generation: cow.active_generation,
                });
            }
        }
        Ok(MountRecovery::Clean)
    }
}

const EXT4_ROOT_INODE: u64 = 2;
const MAX_EXT4_EXTENTS_PER_INODE: usize =
    (EXT4_INODE_BLOCK_BYTES - ExtentHeader::SIZE) / Extent::SIZE;

impl From<Ext4Error> for VfsError {
    fn from(value: Ext4Error) -> Self {
        match value {
            Ext4Error::BufferTooSmall
            | Ext4Error::BadMagic
            | Ext4Error::ChecksumMismatch
            | Ext4Error::InvalidBlockSize
            | Ext4Error::InvalidDescriptor
            | Ext4Error::InvalidExtent
            | Ext4Error::InvalidInode
            | Ext4Error::InvalidDirectoryEntry
            | Ext4Error::InvalidBitmap => VfsError::InvalidArgument,
            Ext4Error::JournalReplayNeeded => VfsError::Busy,
            Ext4Error::NoSpace => VfsError::NoSpace,
            Ext4Error::Device(_) => VfsError::Unsupported,
        }
    }
}

impl<'a> Ext4Backend<'a> {
    fn ext4_inode_number(inode: InodeId) -> u64 {
        if inode == InodeId::ROOT {
            EXT4_ROOT_INODE
        } else {
            inode.raw()
        }
    }

    fn vfs_inode_id(ext4_inode: u64) -> InodeId {
        if ext4_inode == EXT4_ROOT_INODE {
            InodeId::ROOT
        } else {
            InodeId::new(ext4_inode)
        }
    }

    fn sectors_per_block(&self) -> Result<u64, Ext4Error> {
        Ok(self.superblock.block_size()? as u64 / self.device.sector_size() as u64)
    }

    fn read_block_into(&self, block: u64, buffer: &mut [u8]) -> Result<(), Ext4Error> {
        let block_size = self.superblock.block_size()? as usize;
        if buffer.len() < block_size || block_size > MAX_METADATA_BLOCK_BYTES {
            return Err(Ext4Error::BufferTooSmall);
        }
        let sectors_per_block = self.sectors_per_block()?;
        self.device.read_sectors(
            block.saturating_mul(sectors_per_block),
            &mut buffer[..block_size],
        )?;
        Ok(())
    }

    fn read_block_group_descriptor(&self, group: u32) -> Result<BlockGroupDescriptor, Ext4Error> {
        let block_size = self.superblock.block_size()? as usize;
        if block_size > MAX_METADATA_BLOCK_BYTES {
            return Err(Ext4Error::InvalidBlockSize);
        }
        let descriptor_size = self
            .superblock
            .descriptor_size
            .max(BlockGroupDescriptor::MIN_SIZE as u16) as usize;
        if descriptor_size > BlockGroupDescriptor::FULL_SIZE {
            return Err(Ext4Error::InvalidDescriptor);
        }
        let descriptors_per_block = block_size / descriptor_size;
        if descriptors_per_block == 0 {
            return Err(Ext4Error::InvalidDescriptor);
        }
        let descriptor_table_block = self.superblock.first_data_block as u64 + 1;
        let descriptor_block =
            descriptor_table_block + (group as usize / descriptors_per_block) as u64;
        let descriptor_offset = (group as usize % descriptors_per_block) * descriptor_size;
        let mut block = [0u8; MAX_METADATA_BLOCK_BYTES];
        self.read_block_into(descriptor_block, &mut block)?;
        BlockGroupDescriptor::parse(&block[descriptor_offset..descriptor_offset + descriptor_size])
    }

    fn read_inode_record(&self, inode: InodeId) -> Result<InodeRecord, Ext4Error> {
        let ext4_inode = Self::ext4_inode_number(inode);
        if ext4_inode == 0 || ext4_inode > self.superblock.inodes_count as u64 {
            return Err(Ext4Error::InvalidInode);
        }
        let inode_index = ext4_inode - 1;
        let inodes_per_group = self.superblock.inodes_per_group.max(1) as u64;
        let group = (inode_index / inodes_per_group) as u32;
        let index_in_group = inode_index % inodes_per_group;
        let descriptor = self.read_block_group_descriptor(group)?;
        let inode_size = self.superblock.inode_size.max(EXT4_GOOD_OLD_INODE_SIZE) as usize;
        let block_size = self.superblock.block_size()? as usize;
        if inode_size > MAX_METADATA_BLOCK_BYTES || block_size > MAX_METADATA_BLOCK_BYTES {
            return Err(Ext4Error::InvalidInode);
        }
        let byte_offset = index_in_group.saturating_mul(inode_size as u64);
        let inode_block = descriptor.inode_table + byte_offset / block_size as u64;
        let offset_in_block = (byte_offset % block_size as u64) as usize;
        let mut block = [0u8; MAX_METADATA_BLOCK_BYTES];
        self.read_block_into(inode_block, &mut block)?;
        if offset_in_block + inode_size > block_size {
            return Err(Ext4Error::InvalidInode);
        }
        InodeRecord::parse(&block[offset_in_block..offset_in_block + inode_size])
    }

    fn data_block_for(
        &self,
        inode: &InodeRecord,
        logical_block: u64,
    ) -> Result<Option<u64>, Ext4Error> {
        if inode.uses_extents() {
            let tree = ExtentTree::<MAX_EXT4_EXTENTS_PER_INODE>::parse_leaf(&inode.block)?;
            let mut idx = 0usize;
            while idx < tree.header.entries as usize {
                let extent = tree.extents[idx].ok_or(Ext4Error::InvalidExtent)?;
                let start = extent.logical_block as u64;
                let end = start.saturating_add(extent.len as u64);
                if logical_block >= start && logical_block < end {
                    return Ok(Some(extent.start + logical_block - start));
                }
                idx += 1;
            }
            return Ok(None);
        }

        if logical_block < 12 {
            let offset = logical_block as usize * 4;
            let block = LeCursor::new(&inode.block).u32_at(offset)? as u64;
            if block == 0 {
                Ok(None)
            } else {
                Ok(Some(block))
            }
        } else {
            Err(Ext4Error::InvalidExtent)
        }
    }

    fn read_inode_data(
        &self,
        inode: &InodeRecord,
        offset: u64,
        buffer: &mut [u8],
    ) -> Result<usize, Ext4Error> {
        if offset >= inode.size || buffer.is_empty() {
            return Ok(0);
        }
        let block_size = self.superblock.block_size()? as usize;
        if block_size > MAX_METADATA_BLOCK_BYTES {
            return Err(Ext4Error::InvalidBlockSize);
        }
        let available = (inode.size - offset) as usize;
        let target = min(buffer.len(), available);
        let mut done = 0usize;
        let mut block = [0u8; MAX_METADATA_BLOCK_BYTES];
        while done < target {
            let absolute = offset + done as u64;
            let logical_block = absolute / block_size as u64;
            let block_offset = (absolute % block_size as u64) as usize;
            let chunk = min(target - done, block_size - block_offset);
            if let Some(physical_block) = self.data_block_for(inode, logical_block)? {
                self.read_block_into(physical_block, &mut block)?;
                buffer[done..done + chunk]
                    .copy_from_slice(&block[block_offset..block_offset + chunk]);
            } else {
                buffer[done..done + chunk].fill(0);
            }
            done += chunk;
        }
        Ok(done)
    }

    fn find_child(
        &self,
        parent: InodeId,
        name: &str,
    ) -> Result<Option<(InodeId, DirectoryFileType)>, VfsError> {
        let parent_record = self.read_inode_record(parent)?;
        if parent_record.mode.kind() != InodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        let mut offset = 0u64;
        let mut block = [0u8; MAX_METADATA_BLOCK_BYTES];
        while offset < parent_record.size {
            let read = self.read_inode_data(&parent_record, offset, &mut block)?;
            if read == 0 {
                break;
            }
            let mut cursor = 0usize;
            while cursor + EXT4_DIR_ENTRY_HEADER_LEN <= read {
                let entry = DirectoryEntry::parse(&block[cursor..read])?;
                if entry.inode != 0 && entry.name == name.as_bytes() {
                    return Ok(Some((
                        Self::vfs_inode_id(entry.inode as u64),
                        entry.file_type,
                    )));
                }
                if entry.record_len == 0 {
                    return Err(VfsError::InvalidArgument);
                }
                cursor += entry.record_len as usize;
            }
            offset += read as u64;
        }
        Ok(None)
    }
}

impl<'a> FileSystem for Ext4Backend<'a> {
    fn root_inode(&self) -> InodeId {
        InodeId::ROOT
    }

    fn super_block(&self) -> VfsSuperBlock {
        let mut superblock = self.superblock.to_vfs_superblock();
        superblock.read_only = true;
        superblock
    }

    fn lookup(&self, path: Path<'_>) -> Result<InodeMetadata, VfsError> {
        let mut inode = self.root_inode();
        let mut components = path.components();
        while let Some(component) = components.next() {
            inode = self
                .find_child(inode, component)?
                .ok_or(VfsError::NotFound)?
                .0;
        }
        self.lookup_inode(inode)
    }

    fn lookup_inode(&self, inode: InodeId) -> Result<InodeMetadata, VfsError> {
        let record = self.read_inode_record(inode)?;
        Ok(record.metadata(inode))
    }

    fn open(
        &self,
        path: Path<'_>,
        flags: OpenFlags,
        credentials: Credentials,
    ) -> Result<File, VfsError> {
        if flags.access_mode().can_write()
            || flags.contains(OpenFlags::CREATE)
            || flags.contains(OpenFlags::TRUNCATE)
        {
            return Err(VfsError::ReadOnly);
        }
        let metadata = self.lookup(path)?;
        if flags.contains(OpenFlags::DIRECTORY) && metadata.kind != InodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        if !metadata.permissions.allows(credentials, AccessMode::Read) {
            return Err(VfsError::PermissionDenied);
        }
        Ok(File::with_flags(metadata.id, flags).with_path(path))
    }

    fn pread(&self, file: &File, buffer: &mut [u8], offset: u64) -> Result<usize, VfsError> {
        let inode = self.read_inode_record(file.inode())?;
        if inode.mode.kind() == InodeKind::Directory {
            return Err(VfsError::IsDirectory);
        }
        self.read_inode_data(&inode, offset, buffer)
            .map_err(Into::into)
    }

    fn readdir(
        &self,
        path: Path<'_>,
        offset: usize,
        entries: &mut [DirEntry],
    ) -> Result<usize, VfsError> {
        let metadata = self.lookup(path)?;
        self.readdir_inode(metadata.id, offset, entries)
    }

    fn readdir_inode(
        &self,
        inode: InodeId,
        offset: usize,
        entries: &mut [DirEntry],
    ) -> Result<usize, VfsError> {
        let record = self.read_inode_record(inode)?;
        if record.mode.kind() != InodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        let mut file_offset = 0u64;
        let mut seen = 0usize;
        let mut written = 0usize;
        let mut block = [0u8; MAX_METADATA_BLOCK_BYTES];
        while file_offset < record.size && written < entries.len() {
            let read = self.read_inode_data(&record, file_offset, &mut block)?;
            if read == 0 {
                break;
            }
            let mut cursor = 0usize;
            while cursor + EXT4_DIR_ENTRY_HEADER_LEN <= read && written < entries.len() {
                let entry = DirectoryEntry::parse(&block[cursor..read])?;
                if entry.inode != 0 {
                    if seen >= offset {
                        let name =
                            str::from_utf8(entry.name).map_err(|_| VfsError::InvalidArgument)?;
                        entries[written] = DirEntry::new(
                            Self::vfs_inode_id(entry.inode as u64),
                            entry.file_type.kind(),
                            name,
                        )?;
                        written += 1;
                    }
                    seen += 1;
                }
                if entry.record_len == 0 {
                    return Err(VfsError::InvalidArgument);
                }
                cursor += entry.record_len as usize;
            }
            file_offset += read as u64;
        }
        Ok(written)
    }

    fn pwrite(&self, _file: &File, _data: &[u8], _offset: u64) -> Result<usize, VfsError> {
        Err(VfsError::ReadOnly)
    }

    fn mkdir(
        &self,
        _path: Path<'_>,
        _mode: Permissions,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::ReadOnly)
    }

    fn rmdir(&self, _path: Path<'_>, _credentials: Credentials) -> Result<(), VfsError> {
        Err(VfsError::ReadOnly)
    }

    fn unlink(&self, _path: Path<'_>, _credentials: Credentials) -> Result<(), VfsError> {
        Err(VfsError::ReadOnly)
    }

    fn rename(
        &self,
        _old_path: Path<'_>,
        _new_path: Path<'_>,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::ReadOnly)
    }

    fn link(
        &self,
        _old_path: Path<'_>,
        _new_path: Path<'_>,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::ReadOnly)
    }

    fn symlink(
        &self,
        _target: &str,
        _link_path: Path<'_>,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::ReadOnly)
    }

    fn chmod(
        &self,
        _path: Path<'_>,
        _mode: u16,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::ReadOnly)
    }

    fn chown(
        &self,
        _path: Path<'_>,
        _uid: u16,
        _gid: u16,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::ReadOnly)
    }

    fn truncate(
        &self,
        _path: Path<'_>,
        _size: u64,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::ReadOnly)
    }

    fn ftruncate(
        &self,
        _file: &File,
        _size: u64,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::ReadOnly)
    }
}

const fn align_up_u64(value: u64, align: u64) -> u64 {
    if align <= 1 {
        return value;
    }
    let remainder = value % align;
    if remainder == 0 {
        value
    } else {
        value + (align - remainder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superblock_round_trips_core_fields() {
        let sb = Ext4Superblock {
            inodes_count: 42,
            blocks_count: 0x1_0000_1234,
            reserved_blocks_count: 8,
            free_blocks_count: 0x1_0000_0001,
            free_inodes_count: 7,
            first_data_block: 0,
            log_block_size: 2,
            log_cluster_size: 2,
            blocks_per_group: 32768,
            clusters_per_group: 32768,
            inodes_per_group: 1024,
            mount_time: 1,
            write_time: 2,
            mount_count: 3,
            max_mount_count: 20,
            state: 1,
            errors: 0,
            minor_rev_level: 0,
            last_check: 4,
            check_interval: 5,
            creator_os: 0,
            rev_level: 1,
            first_inode: 11,
            inode_size: EXT4_DYNAMIC_INODE_SIZE,
            block_group_nr: 0,
            feature_compat: 0,
            feature_incompat: EXT4_FEATURE_INCOMPAT_EXTENTS | EXT4_FEATURE_INCOMPAT_64BIT,
            feature_ro_compat: EXT4_FEATURE_RO_COMPAT_METADATA_CSUM,
            uuid: [1; 16],
            volume_name: [2; 16],
            last_mounted: [3; 64],
            algorithm_usage_bitmap: 0,
            descriptor_size: 64,
            checksum_seed: 0x1234,
            checksum: 0xabcd,
        };
        let mut bytes = [0u8; EXT4_SUPERBLOCK_SIZE];
        sb.serialize(&mut bytes).unwrap();
        let parsed = Ext4Superblock::parse(&bytes).unwrap();
        assert_eq!(parsed.blocks_count, sb.blocks_count);
        assert_eq!(parsed.free_blocks_count, sb.free_blocks_count);
        assert_eq!(parsed.block_size().unwrap(), 4096);
        assert!(parsed.extents_enabled());
        assert!(parsed.metadata_checksums_enabled());
    }

    #[test]
    fn extent_tree_serializes_leaf_entries() {
        let mut tree: ExtentTree<4> = ExtentTree::empty();
        tree.append(Extent {
            logical_block: 0,
            len: 16,
            start: 0x1_0000_0000,
        })
        .unwrap();
        let mut bytes = [0u8; 64];
        let len = tree.serialize_leaf(&mut bytes).unwrap();
        let parsed: ExtentTree<4> = ExtentTree::parse_leaf(&bytes[..len]).unwrap();
        assert_eq!(parsed.extents[0].unwrap().start, 0x1_0000_0000);
    }

    #[test]
    fn inode_preserves_modes_links_timestamps_and_inline_symlink() {
        let mut inode = InodeRecord {
            mode: InodeMode::from_kind(InodeKind::Symlink, 0o777),
            uid: 1000,
            gid: 100,
            size: 6,
            access_time: Ext4Timestamp::new(1, 0),
            change_time: Ext4Timestamp::new(2, 0),
            modification_time: Ext4Timestamp::new(3, 0),
            deletion_time: 0,
            links_count: 2,
            blocks_count: 0,
            flags: EXT4_EXTENTS_FL,
            block: [0; EXT4_INODE_BLOCK_BYTES],
            generation: 9,
            file_acl: 0,
            checksum: 0xdead_beef,
        };
        inode.block[..6].copy_from_slice(b"target");
        let mut bytes = [0u8; EXT4_DYNAMIC_INODE_SIZE as usize];
        inode.serialize(&mut bytes).unwrap();
        let parsed = InodeRecord::parse(&bytes).unwrap();
        assert_eq!(parsed.mode.kind(), InodeKind::Symlink);
        assert_eq!(parsed.links_count, 2);
        assert_eq!(parsed.inline_symlink().unwrap(), b"target");
        assert!(parsed.uses_extents());
    }

    #[test]
    fn directory_entry_round_trips_without_allocations() {
        let mut bytes = [0u8; 32];
        let len =
            DirectoryEntry::serialize_into(12, DirectoryFileType::Directory, b"etc", &mut bytes)
                .unwrap();
        let parsed = DirectoryEntry::parse(&bytes[..len]).unwrap();
        assert_eq!(parsed.inode, 12);
        assert_eq!(parsed.name, b"etc");
        assert_eq!(parsed.file_type.kind(), InodeKind::Directory);
    }

    #[test]
    fn bitmap_mutates_and_finds_zero_bits() {
        let mut raw = [0xff, 0b0000_1111];
        let mut map = BitmapMut::new(&mut raw);
        map.set(9, true).unwrap();
        map.set(10, false).unwrap();
        let view = map.as_bitmap();
        assert_eq!(view.is_set(9).unwrap(), true);
        assert_eq!(view.first_zero_from(0, 16), Some(10));
    }

    #[test]
    fn crc32c_matches_known_vector() {
        assert_eq!(crc32c(0, b"123456789"), 0xe306_9283);
    }

    #[test]
    fn cow_promotes_shadow_metadata() {
        let mut cow = CowMetadata {
            active_generation: 1,
            shadow_generation: 2,
            active_root: 10,
            shadow_root: 20,
            flags: 0,
            checksum: 0,
        };
        cow.promote_shadow();
        assert_eq!(cow.active_generation, 2);
        assert_eq!(cow.active_root, 20);
    }
}
