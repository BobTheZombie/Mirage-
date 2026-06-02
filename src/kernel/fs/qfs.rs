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
        vfs::{FileSystem, FsError, SuperBlock, VfsError},
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
const QFS_JOURNAL_SLOT_SECTORS: u64 = 2;
const QFS_JOURNAL_PAYLOAD_SECTOR_OFFSET: u64 = 1;
const QFS_JOURNAL_RECORD_MAGIC: u32 = 0x4a_46_53_51;

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

/// Monotonic QFS journal transaction identifier.
pub type QfsTransactionId = u64;

/// Journal record kind stored in each fixed-size on-disk journal slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum QfsJournalRecordKind {
    Begin = 1,
    MetadataWrite = 2,
    DataWrite = 3,
    Commit = 4,
    Abort = 5,
}

impl QfsJournalRecordKind {
    const fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::Begin),
            2 => Some(Self::MetadataWrite),
            3 => Some(Self::DataWrite),
            4 => Some(Self::Commit),
            5 => Some(Self::Abort),
            _ => None,
        }
    }
}

/// Fixed-width journal descriptor for replay-safe metadata and data updates.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QfsJournalRecord {
    pub sequence: QfsTransactionId,
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

    pub const fn kind(self) -> Option<QfsJournalRecordKind> {
        QfsJournalRecordKind::from_u16(self.record_type)
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
    next_transaction_id: QfsTransactionId,
    next_journal_slot: usize,
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
            next_transaction_id: 1,
            next_journal_slot: 0,
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

    fn child_slot_by_name(&self, parent: InodeId, name: &str) -> Option<usize> {
        let mut idx = 0usize;
        while idx < QFS_MAX_INODE_RECORDS {
            if let Some(record) = self.inodes[idx] {
                if record.parent_inode == parent.raw() && record.name() == name {
                    return Some(idx);
                }
            }
            idx += 1;
        }
        None
    }

    fn inode_slot_by_id(&self, inode: InodeId) -> Option<usize> {
        let mut idx = 0usize;
        while idx < QFS_MAX_INODE_RECORDS {
            if let Some(record) = self.inodes[idx] {
                if record.inode == inode.raw() {
                    return Some(idx);
                }
            }
            idx += 1;
        }
        None
    }

    fn free_inode_slot(&self) -> Option<usize> {
        let mut idx = 0usize;
        while idx < QFS_MAX_INODE_RECORDS {
            if self.inodes[idx].is_none() {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    fn next_inode_id(&self) -> InodeId {
        let mut next = InodeId::ROOT.raw().saturating_add(1);
        let mut idx = 0usize;
        while idx < QFS_MAX_INODE_RECORDS {
            if let Some(record) = self.inodes[idx] {
                if record.inode >= next {
                    next = record.inode.saturating_add(1);
                }
            }
            idx += 1;
        }
        InodeId::new(next)
    }

    fn set_link_count(&mut self, inode: InodeId, links: u16) {
        let mut idx = 0usize;
        while idx < QFS_MAX_INODE_RECORDS {
            if let Some(mut record) = self.inodes[idx] {
                if record.inode == inode.raw() {
                    record.links = links;
                    self.inodes[idx] = Some(record);
                }
            }
            idx += 1;
        }
    }

    fn link_count(&self, inode: InodeId) -> u16 {
        let mut count = 0u16;
        let mut idx = 0usize;
        while idx < QFS_MAX_INODE_RECORDS {
            if let Some(record) = self.inodes[idx] {
                if record.inode == inode.raw() {
                    count = count.saturating_add(1);
                }
            }
            idx += 1;
        }
        count
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
        let transaction_id = self.begin_transaction()?;
        self.journal_write(
            transaction_id,
            QfsJournalRecordKind::MetadataWrite,
            0,
            QFS_SUPERBLOCK_SECTOR,
            &sector,
        )?;
        self.commit_transaction(transaction_id)
    }

    pub fn replay_journal(&self) -> Result<(), VfsError> {
        let Some(device) = self.block_device else {
            return Ok(());
        };
        if device.sector_size() != QFS_SECTOR_SIZE {
            return Err(VfsError::Unsupported);
        }
        let superblock = { self.state.lock().superblock };
        let Some(journal_start) = page_location_sector(&superblock, superblock.journal) else {
            return Ok(());
        };
        let slot_count = journal_slot_count(&superblock);
        if slot_count == 0 {
            return Ok(());
        }
        let mut records = [None; QFS_MAX_JOURNAL_RECORDS];
        let mut max_transaction_id = 0u64;
        let mut slot = 0usize;
        while slot < slot_count {
            let mut sector = [0u8; QFS_SECTOR_SIZE];
            let record_sector = journal_start + slot as u64 * QFS_JOURNAL_SLOT_SECTORS;
            if record_sector + QFS_JOURNAL_PAYLOAD_SECTOR_OFFSET >= device.sector_count() {
                break;
            }
            read_sector(device, record_sector, &mut sector)?;
            if let Some(record) = parse_journal_record(&sector)? {
                max_transaction_id = max_u64(max_transaction_id, record.sequence);
                records[slot] = Some(record);
            }
            slot += 1;
        }

        let mut idx = 0usize;
        while idx < slot_count {
            if let Some(commit) = records[idx] {
                if commit.kind() == Some(QfsJournalRecordKind::Commit)
                    && transaction_has_begin(&records, commit.sequence)
                    && !transaction_has_abort(&records, commit.sequence)
                {
                    apply_transaction_records(device, journal_start, &records, commit.sequence)?;
                }
            }
            idx += 1;
        }
        device.flush().map_err(map_device_error)?;

        let mut state = self.state.lock();
        state.journal = records;
        state.next_transaction_id = max_u64(
            state.next_transaction_id,
            max_transaction_id.saturating_add(1),
        );
        state.next_journal_slot = next_journal_slot(&records, slot_count);
        Ok(())
    }

    pub fn begin_transaction(&self) -> Result<QfsTransactionId, VfsError> {
        if self.read_only {
            return Err(VfsError::ReadOnly);
        }
        let transaction_id = {
            let mut state = self.state.lock();
            let id = state.next_transaction_id;
            state.next_transaction_id = state.next_transaction_id.saturating_add(1);
            id
        };
        self.append_journal_record(
            QfsJournalRecord {
                sequence: transaction_id,
                record_type: QfsJournalRecordKind::Begin as u16,
                target_inode: 0,
                sector: 0,
                sector_count: 0,
                checksum: 0,
                flags: 0,
            },
            None,
        )?;
        Ok(transaction_id)
    }

    pub fn journal_write(
        &self,
        transaction_id: QfsTransactionId,
        kind: QfsJournalRecordKind,
        target_inode: u64,
        sector: u64,
        data: &[u8; QFS_SECTOR_SIZE],
    ) -> Result<(), VfsError> {
        if self.read_only {
            return Err(VfsError::ReadOnly);
        }
        if kind != QfsJournalRecordKind::MetadataWrite && kind != QfsJournalRecordKind::DataWrite {
            return Err(VfsError::InvalidArgument);
        }
        self.append_journal_record(
            QfsJournalRecord {
                sequence: transaction_id,
                record_type: kind as u16,
                target_inode,
                sector,
                sector_count: 1,
                checksum: checksum_sector(data),
                flags: 0,
            },
            Some(data),
        )
    }

    pub fn commit_transaction(&self, transaction_id: QfsTransactionId) -> Result<(), VfsError> {
        if self.read_only {
            return Err(VfsError::ReadOnly);
        }
        let Some(device) = self.block_device else {
            return Ok(());
        };
        if device.sector_size() != QFS_SECTOR_SIZE {
            return Err(VfsError::Unsupported);
        }
        self.append_journal_record(
            QfsJournalRecord {
                sequence: transaction_id,
                record_type: QfsJournalRecordKind::Commit as u16,
                target_inode: 0,
                sector: 0,
                sector_count: 0,
                checksum: 0,
                flags: 0,
            },
            None,
        )?;
        let state = self.state.lock();
        let journal_start = page_location_sector(&state.superblock, state.superblock.journal)
            .ok_or(VfsError::Unsupported)?;
        let records = state.journal;
        drop(state);
        apply_transaction_records(device, journal_start, &records, transaction_id)?;
        device.flush().map_err(map_device_error)
    }

    fn append_journal_record(
        &self,
        record: QfsJournalRecord,
        payload: Option<&[u8; QFS_SECTOR_SIZE]>,
    ) -> Result<(), VfsError> {
        let Some(device) = self.block_device else {
            return Ok(());
        };
        if device.sector_size() != QFS_SECTOR_SIZE {
            return Err(VfsError::Unsupported);
        }
        let mut state = self.state.lock();
        let journal_start = page_location_sector(&state.superblock, state.superblock.journal)
            .ok_or(VfsError::Unsupported)?;
        let slot_count = journal_slot_count(&state.superblock);
        if slot_count == 0 {
            return Err(VfsError::NoSpace);
        }
        let slot = state.next_journal_slot % slot_count;
        let record_sector = journal_start + slot as u64 * QFS_JOURNAL_SLOT_SECTORS;
        if record_sector + QFS_JOURNAL_PAYLOAD_SECTOR_OFFSET >= device.sector_count() {
            return Err(VfsError::NoSpace);
        }
        let mut sector = [0u8; QFS_SECTOR_SIZE];
        serialize_journal_record(&record, &mut sector)?;
        write_sector(device, record_sector, &sector)?;
        if let Some(payload) = payload {
            write_sector(
                device,
                record_sector + QFS_JOURNAL_PAYLOAD_SECTOR_OFFSET,
                payload,
            )?;
        }
        state.journal[slot] = Some(record);
        state.next_journal_slot = (slot + 1) % slot_count;
        Ok(())
    }

    fn persist_inode_table(&self, transaction_id: QfsTransactionId) -> Result<(), FsError> {
        let Some(device) = self.block_device else {
            return Ok(());
        };
        if device.sector_size() != QFS_SECTOR_SIZE {
            return Err(FsError::Unsupported);
        }
        let (superblock, inodes) = {
            let state = self.state.lock();
            (state.superblock, state.inodes)
        };
        let sector_number = page_location_sector(&superblock, superblock.inode_table)
            .ok_or(FsError::Unsupported)?;
        let mut sector = [0u8; QFS_SECTOR_SIZE];
        serialize_inode_records(&inodes, &mut sector)?;
        self.journal_write(
            transaction_id,
            QfsJournalRecordKind::MetadataWrite,
            0,
            sector_number,
            &sector,
        )
    }

    fn commit_inode_transaction(&self, transaction_id: QfsTransactionId) -> Result<(), FsError> {
        self.persist_inode_table(transaction_id)?;
        self.commit_transaction(transaction_id)
    }

    fn parent_and_name(
        &self,
        path: Path<'_>,
    ) -> Result<(InodeId, [u8; QFS_NAME_BYTES], u8), FsError> {
        if path.is_root() {
            return Err(FsError::InvalidArgument);
        }
        let mut components = path.components();
        let mut current = InodeId::ROOT;
        let mut pending = components.next().ok_or(FsError::InvalidArgument)?;
        while let Some(next) = components.next() {
            let state = self.state.lock();
            let parent = state.inode_by_id(current).ok_or(FsError::NotFound)?;
            if decode_inode_kind(parent.kind) != InodeKind::Directory {
                return Err(FsError::NotDirectory);
            }
            current = InodeId::new(
                state
                    .child_by_name(current, pending)
                    .ok_or(FsError::NotFound)?
                    .inode,
            );
            pending = next;
        }
        if pending.len() > QFS_NAME_BYTES {
            return Err(FsError::NameTooLong);
        }
        let mut name = [0u8; QFS_NAME_BYTES];
        name[..pending.len()].copy_from_slice(pending.as_bytes());
        Ok((current, name, pending.len() as u8))
    }

    fn data_sector_for_offset(
        state: &QfsState,
        record: QfsInodeRecord,
        offset: u64,
    ) -> Option<(u64, usize)> {
        let page_bytes = state.superblock.page_sectors as u64 * QFS_SECTOR_SIZE as u64;
        if page_bytes == 0 {
            return None;
        }
        let logical_page = offset / page_bytes;
        let offset_in_page = offset % page_bytes;
        let mut idx = if record.chapter_count == 0 {
            0usize
        } else {
            record.first_chapter as usize
        };
        let end = if record.chapter_count == 0 {
            QFS_MAX_CHAPTER_INDEX_ENTRIES
        } else {
            min(
                QFS_MAX_CHAPTER_INDEX_ENTRIES,
                record.first_chapter.saturating_add(record.chapter_count) as usize,
            )
        };
        while idx < end {
            if let Some(entry) = state.chapter_index[idx] {
                let in_range = logical_page >= entry.logical_page
                    && logical_page < entry.logical_page.saturating_add(entry.page_count as u64);
                if entry.inode == record.inode && in_range {
                    let page = entry
                        .first_page
                        .saturating_add((logical_page - entry.logical_page) as u16);
                    if let Some(page_sector) = page_location_sector(
                        &state.superblock,
                        QfsPageLocation::new(entry.book_id, page),
                    ) {
                        return Some((
                            page_sector + offset_in_page / QFS_SECTOR_SIZE as u64,
                            (offset_in_page % QFS_SECTOR_SIZE as u64) as usize,
                        ));
                    }
                }
            }
            idx += 1;
        }
        None
    }

    fn write_record_data(
        &self,
        transaction_id: QfsTransactionId,
        record: QfsInodeRecord,
        data: &[u8],
        offset: u64,
    ) -> Result<usize, FsError> {
        let Some(device) = self.block_device else {
            return Err(FsError::NoSpace);
        };
        if device.sector_size() != QFS_SECTOR_SIZE {
            return Err(FsError::Unsupported);
        }
        let mut written = 0usize;
        while written < data.len() {
            let absolute = offset.saturating_add(written as u64);
            let (sector_number, sector_offset) = {
                let state = self.state.lock();
                Self::data_sector_for_offset(&state, record, absolute).ok_or(FsError::NoSpace)?
            };
            let mut sector = [0u8; QFS_SECTOR_SIZE];
            read_sector(device, sector_number, &mut sector)?;
            let to_copy = min(data.len() - written, QFS_SECTOR_SIZE - sector_offset);
            sector[sector_offset..sector_offset + to_copy]
                .copy_from_slice(&data[written..written + to_copy]);
            self.journal_write(
                transaction_id,
                QfsJournalRecordKind::DataWrite,
                record.inode,
                sector_number,
                &sector,
            )?;
            written += to_copy;
        }
        Ok(written)
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
        self.replay_journal()
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
        if offset >= record.size || buffer.is_empty() {
            return Ok(0);
        }
        let available = min((record.size - offset) as usize, buffer.len());
        let inline_len = min(record.inline_data_len as usize, QFS_INLINE_DATA_BYTES);
        if (offset as usize) < inline_len {
            let to_copy = min(available, inline_len - offset as usize);
            let start = offset as usize;
            buffer[..to_copy].copy_from_slice(&record.inline_data[start..start + to_copy]);
            return Ok(to_copy);
        }
        let Some(device) = self.block_device else {
            return Ok(0);
        };
        if device.sector_size() != QFS_SECTOR_SIZE {
            return Err(FsError::Unsupported);
        }
        let mut read = 0usize;
        drop(state);
        while read < available {
            let absolute = offset.saturating_add(read as u64);
            let (sector_number, sector_offset) = {
                let state = self.state.lock();
                Self::data_sector_for_offset(&state, record, absolute).ok_or(FsError::NoSpace)?
            };
            let mut sector = [0u8; QFS_SECTOR_SIZE];
            read_sector(device, sector_number, &mut sector)?;
            let to_copy = min(available - read, QFS_SECTOR_SIZE - sector_offset);
            buffer[read..read + to_copy]
                .copy_from_slice(&sector[sector_offset..sector_offset + to_copy]);
            read += to_copy;
        }
        Ok(read)
    }

    fn pwrite(&self, file: &File, data: &[u8], offset: u64) -> Result<usize, FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        if data.is_empty() {
            return Ok(0);
        }
        let initial_record = {
            let state = self.state.lock();
            let record = state.inode_by_id(file.inode()).ok_or(FsError::NotFound)?;
            if decode_inode_kind(record.kind) == InodeKind::Directory {
                return Err(FsError::IsDirectory);
            }
            record
        };
        if offset.saturating_add(data.len() as u64) > QFS_INLINE_DATA_BYTES as u64 {
            if self.block_device.is_none() {
                return Err(FsError::NoSpace);
            }
            let mut checked = if (offset as usize) < QFS_INLINE_DATA_BYTES {
                QFS_INLINE_DATA_BYTES - offset as usize
            } else {
                0usize
            };
            while checked < data.len() {
                let absolute = offset.saturating_add(checked as u64);
                let state = self.state.lock();
                Self::data_sector_for_offset(&state, initial_record, absolute)
                    .ok_or(FsError::NoSpace)?;
                checked = checked.saturating_add(QFS_SECTOR_SIZE);
            }
        }
        let transaction_id = self.begin_transaction()?;
        let mut wrote_inline = 0usize;
        let record = {
            let mut state = self.state.lock();
            let slot = state
                .inode_slot_by_id(file.inode())
                .ok_or(FsError::NotFound)?;
            let mut record = state.inodes[slot].ok_or(FsError::NotFound)?;
            if (offset as usize) < QFS_INLINE_DATA_BYTES {
                wrote_inline = min(data.len(), QFS_INLINE_DATA_BYTES - offset as usize);
                let start = offset as usize;
                record.inline_data[start..start + wrote_inline]
                    .copy_from_slice(&data[..wrote_inline]);
                record.inline_data_len =
                    max_u16(record.inline_data_len, (start + wrote_inline) as u16);
            }
            record.size = max_u64(record.size, offset.saturating_add(data.len() as u64));
            state.inodes[slot] = Some(record);
            record
        };
        let mut written = wrote_inline;
        if written < data.len() {
            written += self.write_record_data(
                transaction_id,
                record,
                &data[written..],
                offset.saturating_add(written as u64),
            )?;
        }
        self.commit_inode_transaction(transaction_id)?;
        Ok(written)
    }

    fn mkdir(
        &self,
        path: Path<'_>,
        mode: Permissions,
        credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let (parent, name, name_len) = self.parent_and_name(path)?;
        let transaction_id = self.begin_transaction()?;
        {
            let mut state = self.state.lock();
            let parent_record = state.inode_by_id(parent).ok_or(FsError::NotFound)?;
            if decode_inode_kind(parent_record.kind) != InodeKind::Directory {
                return Err(FsError::NotDirectory);
            }
            if !parent_record.metadata().permissions.allows(
                credentials,
                crate::kernel::fs::permissions::AccessMode::Write,
            ) {
                return Err(FsError::PermissionDenied);
            }
            let name_str = core::str::from_utf8(&name[..name_len as usize]).unwrap_or("");
            if state.child_by_name(parent, name_str).is_some() {
                return Err(FsError::AlreadyExists);
            }
            let slot = state.free_inode_slot().ok_or(FsError::NoSpace)?;
            state.inodes[slot] = Some(QfsInodeRecord {
                inode: state.next_inode_id().raw(),
                parent_inode: parent.raw(),
                kind: encode_inode_kind(InodeKind::Directory),
                name_len,
                mode: mode.bits(),
                uid: credentials.uid,
                gid: credentials.gid,
                links: 1,
                size: 0,
                first_chapter: 0,
                chapter_count: 0,
                name,
                inline_data_len: 0,
                inline_data: [0; QFS_INLINE_DATA_BYTES],
            });
        }
        self.commit_inode_transaction(transaction_id)
    }

    fn rmdir(&self, path: Path<'_>, credentials: Credentials) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let record = self.state.lock().resolve_inode(path)?;
        if record.inode == InodeId::ROOT.raw() {
            return Err(FsError::Busy);
        }
        if decode_inode_kind(record.kind) != InodeKind::Directory {
            return Err(FsError::NotDirectory);
        }
        let transaction_id = self.begin_transaction()?;
        {
            let mut state = self.state.lock();
            let mut idx = 0usize;
            while idx < QFS_MAX_INODE_RECORDS {
                if let Some(child) = state.inodes[idx] {
                    if child.parent_inode == record.inode && child.inode != record.inode {
                        return Err(FsError::Busy);
                    }
                }
                idx += 1;
            }
            let parent = state
                .inode_by_id(InodeId::new(record.parent_inode))
                .ok_or(FsError::NotFound)?;
            if !parent.metadata().permissions.allows(
                credentials,
                crate::kernel::fs::permissions::AccessMode::Write,
            ) {
                return Err(FsError::PermissionDenied);
            }
            let slot = state
                .child_slot_by_name(InodeId::new(record.parent_inode), record.name())
                .ok_or(FsError::NotFound)?;
            state.inodes[slot] = None;
        }
        self.commit_inode_transaction(transaction_id)
    }

    fn unlink(&self, path: Path<'_>, credentials: Credentials) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let record = self.state.lock().resolve_inode(path)?;
        if decode_inode_kind(record.kind) == InodeKind::Directory {
            return Err(FsError::IsDirectory);
        }
        let transaction_id = self.begin_transaction()?;
        {
            let mut state = self.state.lock();
            let parent = state
                .inode_by_id(InodeId::new(record.parent_inode))
                .ok_or(FsError::NotFound)?;
            if !parent.metadata().permissions.allows(
                credentials,
                crate::kernel::fs::permissions::AccessMode::Write,
            ) {
                return Err(FsError::PermissionDenied);
            }
            let slot = state
                .child_slot_by_name(InodeId::new(record.parent_inode), record.name())
                .ok_or(FsError::NotFound)?;
            state.inodes[slot] = None;
            let links = state.link_count(InodeId::new(record.inode));
            state.set_link_count(InodeId::new(record.inode), links);
        }
        self.commit_inode_transaction(transaction_id)
    }

    fn rename(
        &self,
        old_path: Path<'_>,
        new_path: Path<'_>,
        credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let record = self.state.lock().resolve_inode(old_path)?;
        let (new_parent, new_name, new_name_len) = self.parent_and_name(new_path)?;
        let transaction_id = self.begin_transaction()?;
        {
            let mut state = self.state.lock();
            let old_parent = state
                .inode_by_id(InodeId::new(record.parent_inode))
                .ok_or(FsError::NotFound)?;
            let parent = state.inode_by_id(new_parent).ok_or(FsError::NotFound)?;
            if decode_inode_kind(parent.kind) != InodeKind::Directory {
                return Err(FsError::NotDirectory);
            }
            if !old_parent.metadata().permissions.allows(
                credentials,
                crate::kernel::fs::permissions::AccessMode::Write,
            ) || !parent.metadata().permissions.allows(
                credentials,
                crate::kernel::fs::permissions::AccessMode::Write,
            ) {
                return Err(FsError::PermissionDenied);
            }
            let new_name_str =
                core::str::from_utf8(&new_name[..new_name_len as usize]).unwrap_or("");
            if state.child_by_name(new_parent, new_name_str).is_some() {
                return Err(FsError::AlreadyExists);
            }
            let slot = state
                .child_slot_by_name(InodeId::new(record.parent_inode), record.name())
                .ok_or(FsError::NotFound)?;
            let mut updated = state.inodes[slot].ok_or(FsError::NotFound)?;
            updated.parent_inode = new_parent.raw();
            updated.name = new_name;
            updated.name_len = new_name_len;
            state.inodes[slot] = Some(updated);
        }
        self.commit_inode_transaction(transaction_id)
    }

    fn link(
        &self,
        old_path: Path<'_>,
        new_path: Path<'_>,
        credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let source = self.state.lock().resolve_inode(old_path)?;
        if decode_inode_kind(source.kind) == InodeKind::Directory {
            return Err(FsError::IsDirectory);
        }
        let (parent, name, name_len) = self.parent_and_name(new_path)?;
        let transaction_id = self.begin_transaction()?;
        {
            let mut state = self.state.lock();
            let parent_record = state.inode_by_id(parent).ok_or(FsError::NotFound)?;
            if decode_inode_kind(parent_record.kind) != InodeKind::Directory {
                return Err(FsError::NotDirectory);
            }
            if !parent_record.metadata().permissions.allows(
                credentials,
                crate::kernel::fs::permissions::AccessMode::Write,
            ) {
                return Err(FsError::PermissionDenied);
            }
            let name_str = core::str::from_utf8(&name[..name_len as usize]).unwrap_or("");
            if state.child_by_name(parent, name_str).is_some() {
                return Err(FsError::AlreadyExists);
            }
            let slot = state.free_inode_slot().ok_or(FsError::NoSpace)?;
            let mut entry = source;
            entry.parent_inode = parent.raw();
            entry.name = name;
            entry.name_len = name_len;
            entry.links = state
                .link_count(InodeId::new(source.inode))
                .saturating_add(1);
            state.inodes[slot] = Some(entry);
            state.set_link_count(InodeId::new(source.inode), entry.links);
        }
        self.commit_inode_transaction(transaction_id)
    }

    fn symlink(
        &self,
        target: Path<'_>,
        link_path: Path<'_>,
        credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let target_bytes = target.as_str().as_bytes();
        let (parent, name, name_len) = self.parent_and_name(link_path)?;
        let transaction_id = self.begin_transaction()?;
        let record = {
            let mut state = self.state.lock();
            let parent_record = state.inode_by_id(parent).ok_or(FsError::NotFound)?;
            if decode_inode_kind(parent_record.kind) != InodeKind::Directory {
                return Err(FsError::NotDirectory);
            }
            if !parent_record.metadata().permissions.allows(
                credentials,
                crate::kernel::fs::permissions::AccessMode::Write,
            ) {
                return Err(FsError::PermissionDenied);
            }
            let name_str = core::str::from_utf8(&name[..name_len as usize]).unwrap_or("");
            if state.child_by_name(parent, name_str).is_some() {
                return Err(FsError::AlreadyExists);
            }
            let slot = state.free_inode_slot().ok_or(FsError::NoSpace)?;
            let mut inline_data = [0u8; QFS_INLINE_DATA_BYTES];
            let inline_len = min(target_bytes.len(), QFS_INLINE_DATA_BYTES);
            inline_data[..inline_len].copy_from_slice(&target_bytes[..inline_len]);
            let record = QfsInodeRecord {
                inode: state.next_inode_id().raw(),
                parent_inode: parent.raw(),
                kind: encode_inode_kind(InodeKind::Symlink),
                name_len,
                mode: 0o777,
                uid: credentials.uid,
                gid: credentials.gid,
                links: 1,
                size: target_bytes.len() as u64,
                first_chapter: 0,
                chapter_count: 0,
                name,
                inline_data_len: inline_len as u16,
                inline_data,
            };
            state.inodes[slot] = Some(record);
            record
        };
        if target_bytes.len() > QFS_INLINE_DATA_BYTES {
            self.write_record_data(
                transaction_id,
                record,
                &target_bytes[QFS_INLINE_DATA_BYTES..],
                QFS_INLINE_DATA_BYTES as u64,
            )?;
        }
        self.commit_inode_transaction(transaction_id)
    }

    fn readlink(&self, path: Path<'_>, buffer: &mut [u8]) -> Result<usize, FsError> {
        let record = self.state.lock().resolve_inode(path)?;
        if decode_inode_kind(record.kind) != InodeKind::Symlink {
            return Err(FsError::InvalidArgument);
        }
        let file = File::new(
            InodeId::new(record.inode),
            crate::kernel::fs::file::FileMode::ReadOnly,
        );
        self.pread(&file, buffer, 0)
    }

    fn chmod(&self, path: Path<'_>, mode: u16, credentials: Credentials) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let record = self.state.lock().resolve_inode(path)?;
        if !credentials.is_kernel && credentials.uid != record.uid {
            return Err(FsError::PermissionDenied);
        }
        let transaction_id = self.begin_transaction()?;
        {
            let mut state = self.state.lock();
            let mut idx = 0usize;
            while idx < QFS_MAX_INODE_RECORDS {
                if let Some(mut candidate) = state.inodes[idx] {
                    if candidate.inode == record.inode {
                        candidate.mode = mode & 0o777;
                        state.inodes[idx] = Some(candidate);
                    }
                }
                idx += 1;
            }
        }
        self.commit_inode_transaction(transaction_id)
    }

    fn chown(
        &self,
        path: Path<'_>,
        uid: u16,
        gid: u16,
        credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let record = self.state.lock().resolve_inode(path)?;
        if !credentials.is_kernel && credentials.uid != 0 {
            return Err(FsError::PermissionDenied);
        }
        let transaction_id = self.begin_transaction()?;
        {
            let mut state = self.state.lock();
            let mut idx = 0usize;
            while idx < QFS_MAX_INODE_RECORDS {
                if let Some(mut candidate) = state.inodes[idx] {
                    if candidate.inode == record.inode {
                        candidate.uid = uid;
                        candidate.gid = gid;
                        state.inodes[idx] = Some(candidate);
                    }
                }
                idx += 1;
            }
        }
        self.commit_inode_transaction(transaction_id)
    }

    fn truncate(&self, path: Path<'_>, size: u64, credentials: Credentials) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let record = self.state.lock().resolve_inode(path)?;
        if !record.metadata().permissions.allows(
            credentials,
            crate::kernel::fs::permissions::AccessMode::Write,
        ) {
            return Err(FsError::PermissionDenied);
        }
        let transaction_id = self.begin_transaction()?;
        {
            let mut state = self.state.lock();
            let mut idx = 0usize;
            while idx < QFS_MAX_INODE_RECORDS {
                if let Some(mut candidate) = state.inodes[idx] {
                    if candidate.inode == record.inode {
                        candidate.size = size;
                        candidate.inline_data_len = min(
                            candidate.inline_data_len as usize,
                            min(size as usize, QFS_INLINE_DATA_BYTES),
                        ) as u16;
                        state.inodes[idx] = Some(candidate);
                    }
                }
                idx += 1;
            }
        }
        self.commit_inode_transaction(transaction_id)
    }

    fn ftruncate(&self, file: &File, size: u64, credentials: Credentials) -> Result<(), FsError> {
        let record = self
            .state
            .lock()
            .inode_by_id(file.inode())
            .ok_or(FsError::NotFound)?;
        if !record.metadata().permissions.allows(
            credentials,
            crate::kernel::fs::permissions::AccessMode::Write,
        ) {
            return Err(FsError::PermissionDenied);
        }
        let transaction_id = self.begin_transaction()?;
        {
            let mut state = self.state.lock();
            let mut idx = 0usize;
            while idx < QFS_MAX_INODE_RECORDS {
                if let Some(mut candidate) = state.inodes[idx] {
                    if candidate.inode == file.inode().raw() {
                        candidate.size = size;
                        candidate.inline_data_len = min(
                            candidate.inline_data_len as usize,
                            min(size as usize, QFS_INLINE_DATA_BYTES),
                        ) as u16;
                        state.inodes[idx] = Some(candidate);
                    }
                }
                idx += 1;
            }
        }
        self.commit_inode_transaction(transaction_id)
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

fn serialize_inode_records(
    inodes: &[Option<QfsInodeRecord>; QFS_MAX_INODE_RECORDS],
    sector: &mut [u8; QFS_SECTOR_SIZE],
) -> Result<(), FsError> {
    sector.fill(0);
    let mut idx = 0usize;
    let mut offset = 0usize;
    while idx < inodes.len() && offset + QFS_INODE_RECORD_BYTES <= QFS_SECTOR_SIZE {
        if let Some(record) = inodes[idx] {
            put_u64(sector, offset, record.inode)?;
            put_u64(sector, offset + 8, record.parent_inode)?;
            sector[offset + 16] = record.kind;
            sector[offset + 17] = min(record.name_len as usize, QFS_NAME_BYTES) as u8;
            put_u16(sector, offset + 18, record.mode)?;
            put_u16(sector, offset + 20, record.uid)?;
            put_u16(sector, offset + 22, record.gid)?;
            put_u16(sector, offset + 24, record.links)?;
            put_u64(sector, offset + 32, record.size)?;
            put_u32(sector, offset + 40, record.first_chapter)?;
            put_u32(sector, offset + 44, record.chapter_count)?;
            sector[offset + 48..offset + 80].copy_from_slice(&record.name);
            let inline_len = min(record.inline_data_len as usize, QFS_INLINE_DATA_BYTES);
            put_u16(sector, offset + 80, inline_len as u16)?;
            sector[offset + 82..offset + 82 + inline_len]
                .copy_from_slice(&record.inline_data[..inline_len]);
        }
        idx += 1;
        offset += QFS_INODE_RECORD_BYTES;
    }
    Ok(())
}

fn parse_journal_record(
    sector: &[u8; QFS_SECTOR_SIZE],
) -> Result<Option<QfsJournalRecord>, FsError> {
    let cur = LeCursor::new(sector);
    if cur.u32_at(0)? != QFS_JOURNAL_RECORD_MAGIC {
        return Ok(None);
    }
    let record_type = cur.u16_at(12)?;
    if QfsJournalRecordKind::from_u16(record_type).is_none() {
        return Ok(None);
    }
    let record = QfsJournalRecord {
        sequence: cur.u64_at(4)?,
        record_type,
        sector_count: cur.u16_at(14)?,
        target_inode: cur.u64_at(16)?,
        sector: cur.u64_at(24)?,
        checksum: cur.u32_at(32)?,
        flags: cur.u32_at(36)?,
    };
    if record.sequence == 0 {
        Ok(None)
    } else {
        Ok(Some(record))
    }
}

fn serialize_journal_record(
    record: &QfsJournalRecord,
    sector: &mut [u8; QFS_SECTOR_SIZE],
) -> Result<(), FsError> {
    sector.fill(0);
    put_u32(sector, 0, QFS_JOURNAL_RECORD_MAGIC)?;
    put_u64(sector, 4, record.sequence)?;
    put_u16(sector, 12, record.record_type)?;
    put_u16(sector, 14, record.sector_count)?;
    put_u64(sector, 16, record.target_inode)?;
    put_u64(sector, 24, record.sector)?;
    put_u32(sector, 32, record.checksum)?;
    put_u32(sector, 36, record.flags)?;
    Ok(())
}

fn journal_slot_count(superblock: &QfsSuperblock) -> usize {
    if superblock.journal.page >= superblock.book_pages {
        return 0;
    }
    let sectors_from_journal_page =
        (superblock.book_pages - superblock.journal.page) as u64 * superblock.page_sectors as u64;
    min(
        QFS_MAX_JOURNAL_RECORDS,
        (sectors_from_journal_page / QFS_JOURNAL_SLOT_SECTORS) as usize,
    )
}

fn next_journal_slot(
    records: &[Option<QfsJournalRecord>; QFS_MAX_JOURNAL_RECORDS],
    slot_count: usize,
) -> usize {
    let mut idx = 0usize;
    let mut best_idx = 0usize;
    let mut best_sequence = 0u64;
    while idx < slot_count {
        match records[idx] {
            None => return idx,
            Some(record) if record.sequence >= best_sequence => {
                best_sequence = record.sequence;
                best_idx = idx;
            }
            _ => {}
        }
        idx += 1;
    }
    (best_idx + 1) % slot_count
}

fn transaction_has_begin(
    records: &[Option<QfsJournalRecord>; QFS_MAX_JOURNAL_RECORDS],
    transaction_id: QfsTransactionId,
) -> bool {
    transaction_has_kind(records, transaction_id, QfsJournalRecordKind::Begin)
}

fn transaction_has_abort(
    records: &[Option<QfsJournalRecord>; QFS_MAX_JOURNAL_RECORDS],
    transaction_id: QfsTransactionId,
) -> bool {
    transaction_has_kind(records, transaction_id, QfsJournalRecordKind::Abort)
}

fn transaction_has_kind(
    records: &[Option<QfsJournalRecord>; QFS_MAX_JOURNAL_RECORDS],
    transaction_id: QfsTransactionId,
    kind: QfsJournalRecordKind,
) -> bool {
    let mut idx = 0usize;
    while idx < records.len() {
        if let Some(record) = records[idx] {
            if record.sequence == transaction_id && record.kind() == Some(kind) {
                return true;
            }
        }
        idx += 1;
    }
    false
}

fn apply_transaction_records(
    device: &dyn BlockStorageDevice,
    journal_start: u64,
    records: &[Option<QfsJournalRecord>; QFS_MAX_JOURNAL_RECORDS],
    transaction_id: QfsTransactionId,
) -> Result<(), FsError> {
    let mut idx = 0usize;
    while idx < records.len() {
        if let Some(record) = records[idx] {
            let write_kind = record.kind() == Some(QfsJournalRecordKind::MetadataWrite)
                || record.kind() == Some(QfsJournalRecordKind::DataWrite);
            if record.sequence == transaction_id && write_kind {
                if record.sector_count != 1 || record.sector >= device.sector_count() {
                    return Err(FsError::NoSpace);
                }
                let mut payload = [0u8; QFS_SECTOR_SIZE];
                let payload_sector = journal_start
                    + idx as u64 * QFS_JOURNAL_SLOT_SECTORS
                    + QFS_JOURNAL_PAYLOAD_SECTOR_OFFSET;
                if payload_sector >= device.sector_count() {
                    return Err(FsError::NoSpace);
                }
                read_sector(device, payload_sector, &mut payload)?;
                if checksum_sector(&payload) != record.checksum {
                    return Err(FsError::Unsupported);
                }
                write_sector(device, record.sector, &payload)?;
            }
        }
        idx += 1;
    }
    Ok(())
}

fn checksum_sector(sector: &[u8; QFS_SECTOR_SIZE]) -> u32 {
    let mut checksum = 0u32;
    let mut idx = 0usize;
    while idx < sector.len() {
        checksum = checksum.wrapping_add(sector[idx] as u32);
        idx += 1;
    }
    checksum
}

const fn max_u16(left: u16, right: u16) -> u16 {
    if left >= right {
        left
    } else {
        right
    }
}

const fn max_u64(left: u64, right: u64) -> u64 {
    if left >= right {
        left
    } else {
        right
    }
}

fn map_device_error(error: DeviceError) -> FsError {
    match error {
        DeviceError::NotFound | DeviceError::RegistryFull => FsError::NoSpace,
        DeviceError::Busy => FsError::Busy,
        DeviceError::Unsupported | DeviceError::BufferTooSmall => FsError::Unsupported,
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
