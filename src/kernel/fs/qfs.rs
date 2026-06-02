//! QFS (Quiet File System) block-backed VFS scaffolding.
//!
//! QFS keeps on-disk metadata in fixed-size sectors/pages and mirrors the
//! kernel's heap-free style: mount state is cached in bounded inline tables,
//! mutable state is protected by [`SpinLock`], and all persistent storage access
//! goes through [`BlockStorageDevice`].

use core::cmp::min;

use crate::kernel::{
    device::{BlockStorageDevice, DeviceError},
    fs::{
        file::File,
        inode::{DirEntry, InodeId, InodeKind, InodeMetadata},
        path::Path,
        permissions::{Credentials, Permissions},
        vfs::{FileSystem, FsError, SuperBlock},
    },
    sync::SpinLock,
};

/// QFS volume signature stored at the beginning of sector zero.
pub const QFS_MAGIC: [u8; 8] = *b"MIRQFS\0\0";
/// Initial QFS wire-format version understood by this kernel module.
pub const QFS_VERSION: u16 = 1;
/// QFS devices are addressed in 512-byte logical sectors.
pub const QFS_SECTOR_SIZE: usize = 512;
/// Number of sectors grouped into a QFS page.
pub const QFS_PAGE_SECTORS: u16 = 8;
/// Number of pages grouped into a QFS book.
pub const QFS_BOOK_PAGES: u16 = 64;

/// Maximum number of book headers cached by the in-kernel QFS mount state.
pub const QFS_MAX_BOOKS: usize = 8;
/// Maximum number of book index entries cached by the in-kernel QFS mount state.
pub const QFS_MAX_BOOK_INDEX_ENTRIES: usize = 16;
/// Maximum number of chapter index entries cached by the in-kernel QFS mount state.
pub const QFS_MAX_CHAPTER_INDEX_ENTRIES: usize = 32;
/// Maximum number of inode records cached by the in-kernel QFS mount state.
pub const QFS_MAX_INODE_RECORDS: usize = 32;
/// Maximum number of journal records cached by the in-kernel QFS mount state.
pub const QFS_MAX_JOURNAL_RECORDS: usize = 16;
/// Maximum number of bytes stored inline in one cached QFS inode record.
pub const QFS_INLINE_DATA_BYTES: usize = 128;

/// Maximum byte length of a QFS inode name.
pub const QFS_NAME_BYTES: usize = 32;
/// Sector containing the QFS superblock.
const QFS_SUPERBLOCK_SECTOR: u64 = 0;
/// QFS books begin immediately after the standalone sector-zero superblock.
const QFS_FIRST_BOOK_SECTOR: u64 = 1;
/// One sector is reserved for each book header.
const QFS_BOOK_HEADER_SECTORS: u64 = 1;
/// Fixed sector count reserved for each book's inline index.
pub const QFS_BOOK_INDEX_SECTORS: u64 = 1;
const QFS_SUPERBLOCK_RESERVED_BYTES: usize = 64;
const QFS_BOOK_HEADER_RESERVED_BYTES: usize = 496;
const QFS_BOOK_INDEX_ENTRY_BYTES: usize = 16;
const QFS_INODE_RECORD_BYTES: usize = 192;

/// Book/page address used by sector-zero metadata pointers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsPageLocation {
    pub book_id: u32,
    pub page: u16,
}

impl QfsPageLocation {
    pub const fn new(book_id: u32, page: u16) -> Self {
        Self { book_id, page }
    }

    pub const fn empty() -> Self {
        Self {
            book_id: 0,
            page: 0,
        }
    }
}

/// Logical role assigned to a run of pages in a book index entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum QfsBookRole {
    Free = 0,
    Inode = 1,
    Data = 2,
    Journal = 3,
    FreeMap = 4,
}

impl QfsBookRole {
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Inode,
            2 => Self::Data,
            3 => Self::Journal,
            4 => Self::FreeMap,
            _ => Self::Free,
        }
    }
}

/// QFS sector-zero superblock.
///
/// The on-disk image is always little-endian and exactly one logical sector.
/// Fields are parsed explicitly instead of by transmuting this host struct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsSuperblock {
    pub magic: [u8; 8],
    pub version: u16,
    pub sector_size: u16,
    pub page_sectors: u16,
    pub book_pages: u16,
    pub total_books: u32,
    pub root_inode: u64,
    pub inode_table: QfsPageLocation,
    pub journal: QfsPageLocation,
    pub free_space_bitmap: QfsPageLocation,
    pub flags: u16,
    pub total_sectors: u64,
    pub free_sectors: u64,
    pub reserved: [u8; QFS_SUPERBLOCK_RESERVED_BYTES],
}

impl QfsSuperblock {
    pub const fn empty() -> Self {
        Self {
            magic: QFS_MAGIC,
            version: QFS_VERSION,
            sector_size: QFS_SECTOR_SIZE as u16,
            page_sectors: QFS_PAGE_SECTORS,
            book_pages: QFS_BOOK_PAGES,
            total_books: 0,
            root_inode: InodeId::ROOT.raw(),
            inode_table: QfsPageLocation::empty(),
            journal: QfsPageLocation::empty(),
            free_space_bitmap: QfsPageLocation::empty(),
            flags: 0,
            total_sectors: 0,
            free_sectors: 0,
            reserved: [0; QFS_SUPERBLOCK_RESERVED_BYTES],
        }
    }

    pub fn parse_sector(sector: &[u8; QFS_SECTOR_SIZE]) -> Result<Self, FsError> {
        parse_superblock(sector)
    }

    pub fn write_sector(&self, sector: &mut [u8; QFS_SECTOR_SIZE]) -> Result<(), FsError> {
        serialize_superblock(self, sector)
    }
}

/// Cached header for one QFS book (a large allocation group).
///
/// This occupies the first sector of each book on disk; all integer fields are
/// encoded little-endian.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsBookHeader {
    pub book_id: u32,
    pub chapter_count: u16,
    pub index_entry_count: u16,
    pub checksum: u32,
    pub generation: u32,
    pub reserved: [u8; QFS_BOOK_HEADER_RESERVED_BYTES],
}

impl QfsBookHeader {
    pub const fn empty() -> Self {
        Self {
            book_id: 0,
            chapter_count: 0,
            index_entry_count: 0,
            checksum: 0,
            generation: 0,
            reserved: [0; QFS_BOOK_HEADER_RESERVED_BYTES],
        }
    }

    pub fn parse_sector(sector: &[u8; QFS_SECTOR_SIZE]) -> Result<Self, FsError> {
        parse_book_header(sector)
    }

    pub fn write_sector(&self, sector: &mut [u8; QFS_SECTOR_SIZE]) -> Result<(), FsError> {
        serialize_book_header(self, sector)
    }
}

/// Fixed-width book index entry describing chapter page ownership inside a book.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsBookIndexEntry {
    pub chapter_id: u32,
    pub first_page: u16,
    pub page_count: u16,
    pub role: QfsBookRole,
    pub flags: u8,
    pub reserved: [u8; 6],
}

impl QfsBookIndexEntry {
    pub const fn empty() -> Self {
        Self {
            chapter_id: 0,
            first_page: 0,
            page_count: 0,
            role: QfsBookRole::Free,
            flags: 0,
            reserved: [0; 6],
        }
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, FsError> {
        parse_book_index_entry(bytes, 0)
    }

    pub fn write(&self, bytes: &mut [u8]) -> Result<(), FsError> {
        serialize_book_index_entry(self, bytes, 0)
    }
}

/// Fixed-width chapter index entry mapping logical file ranges to books/pages.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsChapterIndexEntry {
    pub inode: u64,
    pub logical_page: u64,
    pub book_id: u32,
    pub first_page: u16,
    pub page_count: u16,
    pub flags: u32,
}

impl QfsChapterIndexEntry {
    pub const fn empty() -> Self {
        Self {
            inode: 0,
            logical_page: 0,
            book_id: 0,
            first_page: 0,
            page_count: 0,
            flags: 0,
        }
    }
}

/// Fixed-width inode record cached by QFS mount state.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsInodeRecord {
    pub inode: u64,
    pub parent_inode: u64,
    pub kind: u8,
    pub name_len: u8,
    pub mode: u16,
    pub uid: u16,
    pub gid: u16,
    pub links: u16,
    pub size: u64,
    pub first_chapter: u32,
    pub chapter_count: u32,
    pub name: [u8; QFS_NAME_BYTES],
    pub inline_data_len: u16,
    pub inline_data: [u8; QFS_INLINE_DATA_BYTES],
}

impl QfsInodeRecord {
    pub const fn empty() -> Self {
        Self {
            inode: 0,
            parent_inode: 0,
            kind: 0,
            name_len: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            links: 0,
            size: 0,
            first_chapter: 0,
            chapter_count: 0,
            name: [0; QFS_NAME_BYTES],
            inline_data_len: 0,
            inline_data: [0; QFS_INLINE_DATA_BYTES],
        }
    }

    pub const fn root() -> Self {
        Self {
            inode: InodeId::ROOT.raw(),
            parent_inode: InodeId::ROOT.raw(),
            kind: encode_inode_kind(InodeKind::Directory),
            name_len: 0,
            mode: 0o755,
            uid: 0,
            gid: 0,
            links: 1,
            size: 0,
            first_chapter: 0,
            chapter_count: 0,
            name: [0; QFS_NAME_BYTES],
            inline_data_len: 0,
            inline_data: [0; QFS_INLINE_DATA_BYTES],
        }
    }

    pub fn name(&self) -> &str {
        let len = min(self.name_len as usize, QFS_NAME_BYTES);
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }

    fn metadata(&self) -> InodeMetadata {
        InodeMetadata::with_links(
            InodeId::new(self.inode),
            decode_inode_kind(self.kind),
            self.size,
            Permissions::new(self.mode, self.uid, self.gid),
            self.links,
        )
    }
}

/// Fixed-width journal descriptor for replay-safe metadata updates.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsJournalRecord {
    pub sequence: u64,
    pub record_type: u16,
    pub target_inode: u64,
    pub sector: u64,
    pub sector_count: u16,
    pub checksum: u32,
    pub flags: u32,
}

impl QfsJournalRecord {
    pub const fn empty() -> Self {
        Self {
            sequence: 0,
            record_type: 0,
            target_inode: 0,
            sector: 0,
            sector_count: 0,
            checksum: 0,
            flags: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct QfsState {
    mounted: bool,
    superblock: QfsSuperblock,
    book_headers: [Option<QfsBookHeader>; QFS_MAX_BOOKS],
    book_index: [Option<QfsBookIndexEntry>; QFS_MAX_BOOK_INDEX_ENTRIES],
    chapter_index: [Option<QfsChapterIndexEntry>; QFS_MAX_CHAPTER_INDEX_ENTRIES],
    inodes: [Option<QfsInodeRecord>; QFS_MAX_INODE_RECORDS],
    journal: [Option<QfsJournalRecord>; QFS_MAX_JOURNAL_RECORDS],
}

impl QfsState {
    const fn new() -> Self {
        let mut inodes = [None; QFS_MAX_INODE_RECORDS];
        inodes[0] = Some(QfsInodeRecord::root());
        Self {
            mounted: false,
            superblock: QfsSuperblock::empty(),
            book_headers: [None; QFS_MAX_BOOKS],
            book_index: [None; QFS_MAX_BOOK_INDEX_ENTRIES],
            chapter_index: [None; QFS_MAX_CHAPTER_INDEX_ENTRIES],
            inodes,
            journal: [None; QFS_MAX_JOURNAL_RECORDS],
        }
    }

    fn inode_by_id(&self, inode: InodeId) -> Option<QfsInodeRecord> {
        let mut idx = 0usize;
        while idx < QFS_MAX_INODE_RECORDS {
            if let Some(record) = self.inodes[idx] {
                if record.inode == inode.raw() {
                    return Some(record);
                }
            }
            idx += 1;
        }
        None
    }

    fn child_by_name(&self, parent: InodeId, name: &str) -> Option<QfsInodeRecord> {
        let mut idx = 0usize;
        while idx < QFS_MAX_INODE_RECORDS {
            if let Some(record) = self.inodes[idx] {
                if record.parent_inode == parent.raw() && record.name() == name {
                    return Some(record);
                }
            }
            idx += 1;
        }
        None
    }

    fn resolve_inode(&self, path: Path<'_>) -> Result<QfsInodeRecord, FsError> {
        if path.is_root() {
            return self.inode_by_id(InodeId::ROOT).ok_or(FsError::NotFound);
        }

        let mut current = self.inode_by_id(InodeId::ROOT).ok_or(FsError::NotFound)?;
        let mut components = path.components();
        while let Some(component) = components.next() {
            if decode_inode_kind(current.kind) != InodeKind::Directory {
                return Err(FsError::NotDirectory);
            }
            current = self
                .child_by_name(InodeId::new(current.inode), component)
                .ok_or(FsError::NotFound)?;
        }
        Ok(current)
    }

    fn cached_counts(&self) -> (usize, usize, usize, usize, usize) {
        (
            count_options(&self.book_headers),
            count_options(&self.book_index),
            count_options(&self.chapter_index),
            count_options(&self.inodes),
            count_options(&self.journal),
        )
    }
}

/// Heap-free, block-backed QFS filesystem implementation.
pub struct QfsFileSystem {
    state: SpinLock<QfsState>,
    read_only: bool,
    block_device: Option<&'static dyn BlockStorageDevice>,
}

impl QfsFileSystem {
    pub const fn new(read_only: bool) -> Self {
        Self {
            state: SpinLock::new(QfsState::new()),
            read_only,
            block_device: None,
        }
    }

    pub const fn new_on_block_device(
        read_only: bool,
        block_device: &'static dyn BlockStorageDevice,
    ) -> Self {
        Self {
            state: SpinLock::new(QfsState::new()),
            read_only,
            block_device: Some(block_device),
        }
    }

    pub fn block_device(&self) -> Option<&dyn BlockStorageDevice> {
        self.block_device
    }

    pub fn cached_table_counts(&self) -> (usize, usize, usize, usize, usize) {
        self.state.lock().cached_counts()
    }

    pub fn write_superblock_to_block_device(&self) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let Some(device) = self.block_device else {
            return Ok(());
        };
        if device.sector_size() != QFS_SECTOR_SIZE {
            return Err(FsError::Unsupported);
        }
        let mut sector = [0u8; QFS_SECTOR_SIZE];
        self.state.lock().superblock.write_sector(&mut sector)?;
        write_sector(device, QFS_SUPERBLOCK_SECTOR, &sector)
    }

    pub fn refresh_from_block_device(&self) -> Result<(), FsError> {
        let Some(device) = self.block_device else {
            return Ok(());
        };
        if device.sector_size() != QFS_SECTOR_SIZE {
            return Err(FsError::Unsupported);
        }
        if device.sector_count() == 0 {
            return Err(FsError::NoSpace);
        }

        let mut sector = [0u8; QFS_SECTOR_SIZE];
        read_sector(device, QFS_SUPERBLOCK_SECTOR, &mut sector)?;
        let superblock = parse_superblock(&sector)?;
        let mut next = QfsState::new();
        next.mounted = true;
        next.superblock = superblock;

        let books_to_cache = min(superblock.total_books as usize, QFS_MAX_BOOKS);
        let mut book_idx = 0usize;
        while book_idx < books_to_cache {
            let book_start = book_start_sector(&superblock, book_idx as u32);
            if book_start < device.sector_count() {
                sector.fill(0);
                read_sector(device, book_start, &mut sector)?;
                let header = parse_book_header(&sector)?;
                if header.book_id == book_idx as u32 {
                    next.book_headers[book_idx] = Some(header);

                    let index_sector = book_start + QFS_BOOK_HEADER_SECTORS;
                    if index_sector < device.sector_count() {
                        sector.fill(0);
                        read_sector(device, index_sector, &mut sector)?;
                        parse_book_index_entries(
                            &sector,
                            min(
                                header.index_entry_count as usize,
                                QFS_MAX_BOOK_INDEX_ENTRIES,
                            ),
                            &mut next.book_index,
                        )?;
                    }
                }
            }
            book_idx += 1;
        }

        if let Some(inode_sector) = page_location_sector(&superblock, superblock.inode_table) {
            if inode_sector < device.sector_count() {
                sector.fill(0);
                read_sector(device, inode_sector, &mut sector)?;
                parse_inode_records(&sector, &mut next.inodes)?;
            }
        }
        if next.inode_by_id(InodeId::ROOT).is_none() {
            next.inodes[0] = Some(QfsInodeRecord::root());
        }

        *self.state.lock() = next;
        Ok(())
    }
}

impl FileSystem for QfsFileSystem {
    fn root_inode(&self) -> InodeId {
        InodeId::ROOT
    }

    fn super_block(&self) -> SuperBlock {
        let state = self.state.lock();
        let mut super_block = SuperBlock::new(InodeId::new(state.superblock.root_inode));
        super_block.block_size = state.superblock.sector_size as u32;
        super_block.total_blocks = state.superblock.total_sectors;
        super_block.free_blocks = state.superblock.free_sectors;
        super_block.read_only = self.read_only;
        super_block
    }

    fn lookup(&self, path: Path<'_>) -> Result<InodeMetadata, FsError> {
        Ok(self.state.lock().resolve_inode(path)?.metadata())
    }

    fn lookup_inode(&self, inode: InodeId) -> Result<InodeMetadata, FsError> {
        self.state
            .lock()
            .inode_by_id(inode)
            .map(|record| record.metadata())
            .ok_or(FsError::NotFound)
    }

    fn pread(&self, file: &File, buffer: &mut [u8], offset: u64) -> Result<usize, FsError> {
        let state = self.state.lock();
        let record = state.inode_by_id(file.inode()).ok_or(FsError::NotFound)?;
        if decode_inode_kind(record.kind) == InodeKind::Directory {
            return Err(FsError::IsDirectory);
        }
        if offset >= record.size {
            return Ok(0);
        }
        let available = min((record.size - offset) as usize, buffer.len());
        let inline_len = min(record.inline_data_len as usize, QFS_INLINE_DATA_BYTES);
        if offset as usize >= inline_len {
            return Ok(0);
        }
        let to_copy = min(available, inline_len - offset as usize);
        let start = offset as usize;
        buffer[..to_copy].copy_from_slice(&record.inline_data[start..start + to_copy]);
        Ok(to_copy)
    }

    fn pwrite(&self, _file: &File, _data: &[u8], _offset: u64) -> Result<usize, FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        Err(FsError::Unsupported)
    }

    fn fsync(&self, _file: &File) -> Result<(), FsError> {
        if let Some(device) = self.block_device {
            return device.flush().map_err(map_device_error);
        }
        Ok(())
    }

    fn readdir(
        &self,
        path: Path<'_>,
        offset: usize,
        entries: &mut [DirEntry],
    ) -> Result<usize, FsError> {
        let inode = self.state.lock().resolve_inode(path)?.inode;
        self.readdir_inode(InodeId::new(inode), offset, entries)
    }

    fn readdir_inode(
        &self,
        inode: InodeId,
        offset: usize,
        entries: &mut [DirEntry],
    ) -> Result<usize, FsError> {
        let state = self.state.lock();
        let directory = state.inode_by_id(inode).ok_or(FsError::NotFound)?;
        if decode_inode_kind(directory.kind) != InodeKind::Directory {
            return Err(FsError::NotDirectory);
        }

        let mut seen = 0usize;
        let mut written = 0usize;
        let mut idx = 0usize;
        while idx < QFS_MAX_INODE_RECORDS && written < entries.len() {
            if let Some(record) = state.inodes[idx] {
                if record.parent_inode == inode.raw() && record.inode != inode.raw() {
                    if seen >= offset {
                        entries[written] = DirEntry::new(
                            InodeId::new(record.inode),
                            decode_inode_kind(record.kind),
                            record.name(),
                        )?;
                        written += 1;
                    }
                    seen += 1;
                }
            }
            idx += 1;
        }
        Ok(written)
    }

    fn truncate(
        &self,
        _path: Path<'_>,
        _size: u64,
        _credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        Err(FsError::Unsupported)
    }
}

fn count_options<T>(table: &[Option<T>]) -> usize {
    let mut idx = 0usize;
    let mut count = 0usize;
    while idx < table.len() {
        if table[idx].is_some() {
            count += 1;
        }
        idx += 1;
    }
    count
}

fn read_sector(
    device: &dyn BlockStorageDevice,
    sector: u64,
    buffer: &mut [u8; QFS_SECTOR_SIZE],
) -> Result<(), FsError> {
    let bytes = device
        .read_sectors(sector, buffer)
        .map_err(map_device_error)?;
    if bytes == QFS_SECTOR_SIZE {
        Ok(())
    } else {
        Err(FsError::Unsupported)
    }
}

fn write_sector(
    device: &dyn BlockStorageDevice,
    sector: u64,
    buffer: &[u8; QFS_SECTOR_SIZE],
) -> Result<(), FsError> {
    let bytes = device
        .write_sectors(sector, buffer)
        .map_err(map_device_error)?;
    if bytes == QFS_SECTOR_SIZE {
        Ok(())
    } else {
        Err(FsError::Unsupported)
    }
}

#[derive(Clone, Copy)]
struct LeCursor<'a> {
    bytes: &'a [u8],
}

impl<'a> LeCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn u8_at(self, offset: usize) -> Result<u8, FsError> {
        self.bytes.get(offset).copied().ok_or(FsError::Unsupported)
    }

    fn u16_at(self, offset: usize) -> Result<u16, FsError> {
        let bytes = self
            .bytes
            .get(offset..offset.saturating_add(2))
            .ok_or(FsError::Unsupported)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32_at(self, offset: usize) -> Result<u32, FsError> {
        let bytes = self
            .bytes
            .get(offset..offset.saturating_add(4))
            .ok_or(FsError::Unsupported)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64_at(self, offset: usize) -> Result<u64, FsError> {
        let bytes = self
            .bytes
            .get(offset..offset.saturating_add(8))
            .ok_or(FsError::Unsupported)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }
}

fn put_u16(out: &mut [u8], offset: usize, value: u16) -> Result<(), FsError> {
    let dst = out
        .get_mut(offset..offset.saturating_add(2))
        .ok_or(FsError::Unsupported)?;
    dst.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn put_u32(out: &mut [u8], offset: usize, value: u32) -> Result<(), FsError> {
    let dst = out
        .get_mut(offset..offset.saturating_add(4))
        .ok_or(FsError::Unsupported)?;
    dst.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn put_u64(out: &mut [u8], offset: usize, value: u64) -> Result<(), FsError> {
    let dst = out
        .get_mut(offset..offset.saturating_add(8))
        .ok_or(FsError::Unsupported)?;
    dst.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn book_size_sectors(superblock: &QfsSuperblock) -> u64 {
    superblock.page_sectors as u64 * superblock.book_pages as u64
}

fn book_start_sector(superblock: &QfsSuperblock, book_id: u32) -> u64 {
    QFS_FIRST_BOOK_SECTOR + book_id as u64 * book_size_sectors(superblock)
}

fn page_location_sector(superblock: &QfsSuperblock, location: QfsPageLocation) -> Option<u64> {
    if location.book_id >= superblock.total_books || location.page >= superblock.book_pages {
        return None;
    }
    Some(
        book_start_sector(superblock, location.book_id)
            + location.page as u64 * superblock.page_sectors as u64,
    )
}

fn parse_superblock(sector: &[u8; QFS_SECTOR_SIZE]) -> Result<QfsSuperblock, FsError> {
    let cur = LeCursor::new(sector);
    let mut magic = [0u8; 8];
    magic.copy_from_slice(&sector[0..8]);
    if magic != QFS_MAGIC {
        return Err(FsError::InvalidArgument);
    }
    let version = cur.u16_at(8)?;
    if version != QFS_VERSION {
        return Err(FsError::Unsupported);
    }
    let sector_size = cur.u16_at(10)?;
    if sector_size as usize != QFS_SECTOR_SIZE {
        return Err(FsError::Unsupported);
    }
    let page_sectors = cur.u16_at(12)?;
    let book_pages = cur.u16_at(14)?;
    if page_sectors == 0 || book_pages == 0 {
        return Err(FsError::InvalidArgument);
    }

    let mut reserved = [0u8; QFS_SUPERBLOCK_RESERVED_BYTES];
    reserved.copy_from_slice(&sector[64..64 + QFS_SUPERBLOCK_RESERVED_BYTES]);
    Ok(QfsSuperblock {
        magic,
        version,
        sector_size,
        page_sectors,
        book_pages,
        total_books: cur.u32_at(16)?,
        root_inode: cur.u64_at(20)?,
        inode_table: QfsPageLocation::new(cur.u32_at(28)?, cur.u16_at(32)?),
        journal: QfsPageLocation::new(cur.u32_at(34)?, cur.u16_at(38)?),
        free_space_bitmap: QfsPageLocation::new(cur.u32_at(40)?, cur.u16_at(44)?),
        flags: cur.u16_at(46)?,
        total_sectors: cur.u64_at(48)?,
        free_sectors: cur.u64_at(56)?,
        reserved,
    })
}

fn serialize_superblock(
    superblock: &QfsSuperblock,
    sector: &mut [u8; QFS_SECTOR_SIZE],
) -> Result<(), FsError> {
    sector.fill(0);
    sector[0..8].copy_from_slice(&superblock.magic);
    put_u16(sector, 8, superblock.version)?;
    put_u16(sector, 10, superblock.sector_size)?;
    put_u16(sector, 12, superblock.page_sectors)?;
    put_u16(sector, 14, superblock.book_pages)?;
    put_u32(sector, 16, superblock.total_books)?;
    put_u64(sector, 20, superblock.root_inode)?;
    put_u32(sector, 28, superblock.inode_table.book_id)?;
    put_u16(sector, 32, superblock.inode_table.page)?;
    put_u32(sector, 34, superblock.journal.book_id)?;
    put_u16(sector, 38, superblock.journal.page)?;
    put_u32(sector, 40, superblock.free_space_bitmap.book_id)?;
    put_u16(sector, 44, superblock.free_space_bitmap.page)?;
    put_u16(sector, 46, superblock.flags)?;
    put_u64(sector, 48, superblock.total_sectors)?;
    put_u64(sector, 56, superblock.free_sectors)?;
    sector[64..64 + QFS_SUPERBLOCK_RESERVED_BYTES].copy_from_slice(&superblock.reserved);
    Ok(())
}

fn parse_book_header(sector: &[u8; QFS_SECTOR_SIZE]) -> Result<QfsBookHeader, FsError> {
    let cur = LeCursor::new(sector);
    let mut reserved = [0u8; QFS_BOOK_HEADER_RESERVED_BYTES];
    reserved.copy_from_slice(&sector[16..16 + QFS_BOOK_HEADER_RESERVED_BYTES]);
    Ok(QfsBookHeader {
        book_id: cur.u32_at(0)?,
        chapter_count: cur.u16_at(4)?,
        index_entry_count: cur.u16_at(6)?,
        checksum: cur.u32_at(8)?,
        generation: cur.u32_at(12)?,
        reserved,
    })
}

fn serialize_book_header(
    header: &QfsBookHeader,
    sector: &mut [u8; QFS_SECTOR_SIZE],
) -> Result<(), FsError> {
    sector.fill(0);
    put_u32(sector, 0, header.book_id)?;
    put_u16(sector, 4, header.chapter_count)?;
    put_u16(sector, 6, header.index_entry_count)?;
    put_u32(sector, 8, header.checksum)?;
    put_u32(sector, 12, header.generation)?;
    sector[16..16 + QFS_BOOK_HEADER_RESERVED_BYTES].copy_from_slice(&header.reserved);
    Ok(())
}

fn parse_book_index_entries(
    sector: &[u8; QFS_SECTOR_SIZE],
    entry_count: usize,
    entries: &mut [Option<QfsBookIndexEntry>],
) -> Result<(), FsError> {
    let mut idx = 0usize;
    let max_entries = min(
        min(entry_count, entries.len()),
        QFS_SECTOR_SIZE / QFS_BOOK_INDEX_ENTRY_BYTES,
    );
    while idx < max_entries {
        let offset = idx * QFS_BOOK_INDEX_ENTRY_BYTES;
        let entry = parse_book_index_entry(sector, offset)?;
        if entry.chapter_id != 0 || entry.page_count != 0 || entry.role != QfsBookRole::Free {
            entries[idx] = Some(entry);
        }
        idx += 1;
    }
    Ok(())
}

fn parse_book_index_entry(bytes: &[u8], offset: usize) -> Result<QfsBookIndexEntry, FsError> {
    let cur = LeCursor::new(bytes);
    let mut reserved = [0u8; 6];
    let reserved_src = bytes
        .get(offset.saturating_add(10)..offset.saturating_add(16))
        .ok_or(FsError::Unsupported)?;
    reserved.copy_from_slice(reserved_src);
    Ok(QfsBookIndexEntry {
        chapter_id: cur.u32_at(offset)?,
        first_page: cur.u16_at(offset + 4)?,
        page_count: cur.u16_at(offset + 6)?,
        role: QfsBookRole::from_u8(cur.u8_at(offset + 8)?),
        flags: cur.u8_at(offset + 9)?,
        reserved,
    })
}

fn serialize_book_index_entry(
    entry: &QfsBookIndexEntry,
    bytes: &mut [u8],
    offset: usize,
) -> Result<(), FsError> {
    put_u32(bytes, offset, entry.chapter_id)?;
    put_u16(bytes, offset + 4, entry.first_page)?;
    put_u16(bytes, offset + 6, entry.page_count)?;
    *bytes.get_mut(offset + 8).ok_or(FsError::Unsupported)? = entry.role as u8;
    *bytes.get_mut(offset + 9).ok_or(FsError::Unsupported)? = entry.flags;
    let reserved_dst = bytes
        .get_mut(offset.saturating_add(10)..offset.saturating_add(16))
        .ok_or(FsError::Unsupported)?;
    reserved_dst.copy_from_slice(&entry.reserved);
    Ok(())
}

fn parse_inode_records(
    sector: &[u8; QFS_SECTOR_SIZE],
    inodes: &mut [Option<QfsInodeRecord>],
) -> Result<(), FsError> {
    let cur = LeCursor::new(sector);
    let mut idx = 0usize;
    let mut offset = 0usize;
    while idx < inodes.len() && offset + QFS_INODE_RECORD_BYTES <= QFS_SECTOR_SIZE {
        let inode = cur.u64_at(offset)?;
        if inode != 0 {
            let mut name = [0u8; QFS_NAME_BYTES];
            name.copy_from_slice(&sector[offset + 48..offset + 80]);
            let mut inline_data = [0u8; QFS_INLINE_DATA_BYTES];
            let inline_len = min(cur.u16_at(offset + 80)? as usize, QFS_INLINE_DATA_BYTES);
            inline_data[..inline_len]
                .copy_from_slice(&sector[offset + 82..offset + 82 + inline_len]);
            inodes[idx] = Some(QfsInodeRecord {
                inode,
                parent_inode: cur.u64_at(offset + 8)?,
                kind: cur.u8_at(offset + 16)?,
                name_len: cur.u8_at(offset + 17)?,
                mode: cur.u16_at(offset + 18)?,
                uid: cur.u16_at(offset + 20)?,
                gid: cur.u16_at(offset + 22)?,
                links: cur.u16_at(offset + 24)?,
                size: cur.u64_at(offset + 32)?,
                first_chapter: cur.u32_at(offset + 40)?,
                chapter_count: cur.u32_at(offset + 44)?,
                name,
                inline_data_len: inline_len as u16,
                inline_data,
            });
        }
        idx += 1;
        offset += QFS_INODE_RECORD_BYTES;
    }
    Ok(())
}

fn map_device_error(error: DeviceError) -> FsError {
    match error {
        DeviceError::NotFound => FsError::NotFound,
        DeviceError::Busy => FsError::Busy,
        DeviceError::Unsupported | DeviceError::BufferTooSmall => FsError::Unsupported,
        DeviceError::RegistryFull => FsError::NoSpace,
    }
}

const fn encode_inode_kind(kind: InodeKind) -> u8 {
    match kind {
        InodeKind::Directory => 1,
        InodeKind::RegularFile => 2,
        InodeKind::Symlink => 3,
        InodeKind::BlockDevice => 4,
        InodeKind::CharDevice => 5,
        InodeKind::Fifo => 6,
        InodeKind::Socket => 7,
    }
}

fn decode_inode_kind(kind: u8) -> InodeKind {
    match kind {
        1 => InodeKind::Directory,
        3 => InodeKind::Symlink,
        4 => InodeKind::BlockDevice,
        5 => InodeKind::CharDevice,
        6 => InodeKind::Fifo,
        7 => InodeKind::Socket,
        _ => InodeKind::RegularFile,
    }
}
