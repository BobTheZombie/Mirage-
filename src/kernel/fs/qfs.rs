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
const QFS_SUPERBLOCK_SECTOR: u64 = 0;
const QFS_BOOK_HEADER_SECTOR: u64 = 1;
const QFS_INODE_TABLE_SECTOR: u64 = 2;

/// QFS sector-zero superblock.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsSuperblock {
    pub magic: [u8; 8],
    pub version: u16,
    pub sector_size: u16,
    pub page_sectors: u16,
    pub book_pages: u16,
    pub root_inode: u64,
    pub total_sectors: u64,
    pub free_sectors: u64,
    pub book_count: u32,
    pub inode_count: u32,
    pub journal_head: u64,
    pub flags: u32,
    pub reserved: [u8; 48],
}

impl QfsSuperblock {
    pub const fn empty() -> Self {
        Self {
            magic: QFS_MAGIC,
            version: QFS_VERSION,
            sector_size: QFS_SECTOR_SIZE as u16,
            page_sectors: QFS_PAGE_SECTORS,
            book_pages: QFS_BOOK_PAGES,
            root_inode: InodeId::ROOT.raw(),
            total_sectors: 0,
            free_sectors: 0,
            book_count: 0,
            inode_count: 1,
            journal_head: 0,
            flags: 0,
            reserved: [0; 48],
        }
    }
}

/// Cached header for one QFS book (a large allocation group).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsBookHeader {
    pub book_id: u32,
    pub first_sector: u64,
    pub page_count: u16,
    pub free_pages: u16,
    pub index_sector: u64,
    pub checksum: u32,
    pub flags: u32,
}

impl QfsBookHeader {
    pub const fn empty() -> Self {
        Self {
            book_id: 0,
            first_sector: 0,
            page_count: 0,
            free_pages: 0,
            index_sector: 0,
            checksum: 0,
            flags: 0,
        }
    }
}

/// Fixed-width book index entry describing page ownership inside a book.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsBookIndexEntry {
    pub book_id: u32,
    pub first_page: u16,
    pub page_count: u16,
    pub owner_inode: u64,
    pub flags: u32,
}

impl QfsBookIndexEntry {
    pub const fn empty() -> Self {
        Self {
            book_id: 0,
            first_page: 0,
            page_count: 0,
            owner_inode: 0,
            flags: 0,
        }
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

        if device.sector_count() > QFS_BOOK_HEADER_SECTOR {
            sector.fill(0);
            read_sector(device, QFS_BOOK_HEADER_SECTOR, &mut sector)?;
            parse_book_headers(&sector, &mut next.book_headers);
        }
        if device.sector_count() > QFS_INODE_TABLE_SECTOR {
            sector.fill(0);
            read_sector(device, QFS_INODE_TABLE_SECTOR, &mut sector)?;
            parse_inode_records(&sector, &mut next.inodes);
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

fn parse_superblock(sector: &[u8; QFS_SECTOR_SIZE]) -> Result<QfsSuperblock, FsError> {
    let mut magic = [0u8; 8];
    magic.copy_from_slice(&sector[0..8]);
    if magic != QFS_MAGIC {
        return Err(FsError::InvalidArgument);
    }
    let version = read_u16(sector, 8);
    if version != QFS_VERSION {
        return Err(FsError::Unsupported);
    }
    let sector_size = read_u16(sector, 10);
    if sector_size as usize != QFS_SECTOR_SIZE {
        return Err(FsError::Unsupported);
    }

    let mut reserved = [0u8; 48];
    reserved.copy_from_slice(&sector[64..112]);
    Ok(QfsSuperblock {
        magic,
        version,
        sector_size,
        page_sectors: read_u16(sector, 12),
        book_pages: read_u16(sector, 14),
        root_inode: read_u64(sector, 16),
        total_sectors: read_u64(sector, 24),
        free_sectors: read_u64(sector, 32),
        book_count: read_u32(sector, 40),
        inode_count: read_u32(sector, 44),
        journal_head: read_u64(sector, 48),
        flags: read_u32(sector, 56),
        reserved,
    })
}

fn parse_book_headers(sector: &[u8; QFS_SECTOR_SIZE], headers: &mut [Option<QfsBookHeader>]) {
    let mut idx = 0usize;
    let mut offset = 0usize;
    while idx < headers.len() && offset + 32 <= QFS_SECTOR_SIZE {
        let book_id = read_u32(sector, offset);
        let first_sector = read_u64(sector, offset + 4);
        let page_count = read_u16(sector, offset + 12);
        let free_pages = read_u16(sector, offset + 14);
        if book_id != 0 || first_sector != 0 || page_count != 0 || free_pages != 0 {
            headers[idx] = Some(QfsBookHeader {
                book_id,
                first_sector,
                page_count,
                free_pages,
                index_sector: read_u64(sector, offset + 16),
                checksum: read_u32(sector, offset + 24),
                flags: read_u32(sector, offset + 28),
            });
        }
        idx += 1;
        offset += 32;
    }
}

fn parse_inode_records(sector: &[u8; QFS_SECTOR_SIZE], inodes: &mut [Option<QfsInodeRecord>]) {
    let mut idx = 0usize;
    let mut offset = 0usize;
    while idx < inodes.len() && offset + 64 <= QFS_SECTOR_SIZE {
        let inode = read_u64(sector, offset);
        if inode != 0 {
            let mut name = [0u8; QFS_NAME_BYTES];
            name.copy_from_slice(&sector[offset + 48..offset + 80]);
            let mut inline_data = [0u8; QFS_INLINE_DATA_BYTES];
            let inline_len = min(
                read_u16(sector, offset + 80) as usize,
                QFS_INLINE_DATA_BYTES,
            );
            let available = QFS_SECTOR_SIZE.saturating_sub(offset + 82);
            let copy_len = min(inline_len, available);
            inline_data[..copy_len].copy_from_slice(&sector[offset + 82..offset + 82 + copy_len]);
            inodes[idx] = Some(QfsInodeRecord {
                inode,
                parent_inode: read_u64(sector, offset + 8),
                kind: sector[offset + 16],
                name_len: sector[offset + 17],
                mode: read_u16(sector, offset + 18),
                uid: read_u16(sector, offset + 20),
                gid: read_u16(sector, offset + 22),
                links: read_u16(sector, offset + 24),
                size: read_u64(sector, offset + 32),
                first_chapter: read_u32(sector, offset + 40),
                chapter_count: read_u32(sector, offset + 44),
                name,
                inline_data_len: inline_len as u16,
                inline_data,
            });
        }
        idx += 1;
        offset += 192;
    }
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

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}
