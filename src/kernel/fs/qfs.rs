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

pub use super::qfs_format::{
    apply_transaction_records, initialize_image, parse_book_header, parse_book_index_entries,
    parse_inode_records, read_book_header, read_inode_table, read_superblock,
    replay_valid_journal_records, scan_journal, serialize_book_header, serialize_book_index_entry,
    serialize_inode_records, write_book_header, write_inode_table, write_superblock, QfsBookHeader,
    QfsBookIndexEntry, QfsBookRole, QfsChapterIndexEntry, QfsExtentRecord, QfsInodeRecord,
    QfsJournalRecord, QfsJournalRecordKind, QfsObjectMutationState, QfsPageLocation, QfsSuperblock,
    QfsTransactionId, QFS_BOOK_INDEX_ENTRY_BYTES, QFS_BOOK_INDEX_SECTORS, QFS_BOOK_PAGES,
    QFS_INLINE_DATA_BYTES, QFS_MAGIC, QFS_MAX_BOOKS, QFS_MAX_BOOK_INDEX_ENTRIES,
    QFS_MAX_CHAPTER_INDEX_ENTRIES, QFS_MAX_INODE_RECORDS, QFS_MAX_JOURNAL_RECORDS, QFS_NAME_BYTES,
    QFS_OBJECT_HAS_CAPABILITY, QFS_OBJECT_HAS_SIGNATURE, QFS_OBJECT_SERVICE_AWARE,
    QFS_PAGE_SECTORS, QFS_SECTOR_SIZE, QFS_VERSION,
};

use super::qfs_format::{
    book_start_sector, checksum_sector, decode_inode_kind, encode_inode_kind, journal_slot_count,
    max_u16, max_u64, next_journal_slot, page_location_sector, parse_superblock, read_sector,
    serialize_journal_record, write_sector, QFS_BOOK_HEADER_SECTORS,
    QFS_JOURNAL_PAYLOAD_SECTOR_OFFSET, QFS_JOURNAL_SLOT_SECTORS, QFS_SUPERBLOCK_SECTOR,
};

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

    fn normalize_object_metadata(&mut self) {
        let mut idx = 0usize;
        while idx < QFS_MAX_INODE_RECORDS {
            if let Some(mut record) = self.inodes[idx] {
                if record.object_id == 0 {
                    record.object_id = record.inode;
                }
                record.refresh_path_identity();
                self.inodes[idx] = Some(record);
            }
            idx += 1;
        }
    }

    fn mark_all_inode_mutations(&mut self, transaction_id: QfsTransactionId) {
        let mut idx = 0usize;
        while idx < QFS_MAX_INODE_RECORDS {
            if let Some(mut record) = self.inodes[idx] {
                record.mark_mutation(transaction_id);
                self.inodes[idx] = Some(record);
            }
            idx += 1;
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

    pub fn lookup_object_record(&self, path: Path<'_>) -> Result<QfsInodeRecord, FsError> {
        self.state.lock().resolve_inode(path)
    }

    pub fn lookup_inode_record(&self, inode: InodeId) -> Result<QfsInodeRecord, FsError> {
        self.state
            .lock()
            .inode_by_id(inode)
            .ok_or(FsError::NotFound)
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
        let mut records = [None; QFS_MAX_JOURNAL_RECORDS];
        let (slot_count, max_transaction_id) = scan_journal(device, &superblock, &mut records)?;
        if slot_count == 0 {
            return Ok(());
        }
        replay_valid_journal_records(device, &superblock, &records, slot_count)?;
        device.flush().map_err(map_device_error)?;

        let mut replayed_inodes = [None; QFS_MAX_INODE_RECORDS];
        if let Some(inode_sector) = page_location_sector(&superblock, superblock.inode_table) {
            if inode_sector < device.sector_count() {
                let mut sector = [0u8; QFS_SECTOR_SIZE];
                read_sector(device, inode_sector, &mut sector)?;
                parse_inode_records(&sector, &mut replayed_inodes)?;
            }
        }

        let mut state = self.state.lock();
        if replayed_inodes.iter().any(Option::is_some) {
            state.inodes = replayed_inodes;
            state.normalize_object_metadata();
        }
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
        self.state.lock().mark_all_inode_mutations(transaction_id);
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
                next.normalize_object_metadata();
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
            let inode = state.next_inode_id().raw();
            let mut record = QfsInodeRecord {
                inode,
                object_id: inode,
                parent_inode: parent.raw(),
                kind: encode_inode_kind(InodeKind::Directory),
                name_len,
                metadata_flags: QFS_OBJECT_SERVICE_AWARE,
                mode: mode.bits(),
                uid: credentials.uid,
                gid: credentials.gid,
                links: 1,
                name,
                ..QfsInodeRecord::empty()
            };
            record.refresh_path_identity();
            state.inodes[slot] = Some(record);
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
            updated.refresh_path_identity();
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
            entry.refresh_path_identity();
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
        target: &str,
        link_path: Path<'_>,
        credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let target_bytes = target.as_bytes();
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
            let inode = state.next_inode_id().raw();
            let mut record = QfsInodeRecord {
                inode,
                object_id: inode,
                parent_inode: parent.raw(),
                kind: encode_inode_kind(InodeKind::Symlink),
                name_len,
                metadata_flags: QFS_OBJECT_SERVICE_AWARE,
                mode: 0o777,
                uid: credentials.uid,
                gid: credentials.gid,
                links: 1,
                size: target_bytes.len() as u64,
                name,
                inline_data_len: inline_len as u16,
                inline_data,
                ..QfsInodeRecord::empty()
            };
            record.refresh_path_identity();
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

fn map_device_error(error: DeviceError) -> FsError {
    match error {
        DeviceError::NotFound | DeviceError::RegistryFull => FsError::NoSpace,
        DeviceError::Busy => FsError::Busy,
        DeviceError::Unsupported | DeviceError::BufferTooSmall => FsError::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::kernel::fs::{file::FileMode, permissions::AccessMode};

    struct MemBlockDevice {
        sectors: Mutex<Vec<[u8; QFS_SECTOR_SIZE]>>,
    }

    impl MemBlockDevice {
        fn new(sector_count: usize) -> Self {
            Self {
                sectors: Mutex::new(vec![[0; QFS_SECTOR_SIZE]; sector_count]),
            }
        }

        fn read_sector_copy(&self, sector: u64) -> [u8; QFS_SECTOR_SIZE] {
            self.sectors.lock().unwrap()[sector as usize]
        }

        fn write_sector_copy(&self, sector: u64, data: &[u8; QFS_SECTOR_SIZE]) {
            self.sectors.lock().unwrap()[sector as usize] = *data;
        }
    }

    impl BlockStorageDevice for MemBlockDevice {
        fn sector_size(&self) -> usize {
            QFS_SECTOR_SIZE
        }

        fn sector_count(&self) -> u64 {
            self.sectors.lock().unwrap().len() as u64
        }

        fn read_sectors(&self, first_sector: u64, buffer: &mut [u8]) -> Result<usize, DeviceError> {
            if buffer.len() % QFS_SECTOR_SIZE != 0 {
                return Err(DeviceError::BufferTooSmall);
            }
            let sectors = buffer.len() / QFS_SECTOR_SIZE;
            let guard = self.sectors.lock().unwrap();
            if first_sector as usize + sectors > guard.len() {
                return Err(DeviceError::NotFound);
            }
            let mut idx = 0usize;
            while idx < sectors {
                let start = idx * QFS_SECTOR_SIZE;
                buffer[start..start + QFS_SECTOR_SIZE]
                    .copy_from_slice(&guard[first_sector as usize + idx]);
                idx += 1;
            }
            Ok(buffer.len())
        }

        fn write_sectors(&self, first_sector: u64, data: &[u8]) -> Result<usize, DeviceError> {
            if data.len() % QFS_SECTOR_SIZE != 0 {
                return Err(DeviceError::BufferTooSmall);
            }
            let sectors = data.len() / QFS_SECTOR_SIZE;
            let mut guard = self.sectors.lock().unwrap();
            if first_sector as usize + sectors > guard.len() {
                return Err(DeviceError::NotFound);
            }
            let mut idx = 0usize;
            while idx < sectors {
                let start = idx * QFS_SECTOR_SIZE;
                guard[first_sector as usize + idx]
                    .copy_from_slice(&data[start..start + QFS_SECTOR_SIZE]);
                idx += 1;
            }
            Ok(data.len())
        }

        fn flush(&self) -> Result<(), DeviceError> {
            Ok(())
        }

        fn discard(&self, first_sector: u64, sector_count: u64) -> Result<(), DeviceError> {
            let mut zeroes = vec![0u8; sector_count as usize * QFS_SECTOR_SIZE];
            self.write_sectors(first_sector, &mut zeroes).map(|_| ())
        }
    }

    fn path(raw: &str) -> Path<'_> {
        Path::new(raw).unwrap()
    }

    fn test_superblock(total_books: u32) -> QfsSuperblock {
        QfsSuperblock {
            total_books,
            total_sectors: 1 + total_books as u64 * QFS_BOOK_PAGES as u64 * QFS_PAGE_SECTORS as u64,
            free_sectors: 123,
            inode_table: QfsPageLocation::new(0, 1),
            journal: QfsPageLocation::new(0, 8),
            free_space_bitmap: QfsPageLocation::new(0, 2),
            flags: 0x55aa,
            ..QfsSuperblock::empty()
        }
    }

    fn named_inode(
        inode: u64,
        parent_inode: u64,
        kind: InodeKind,
        name: &str,
        mode: u16,
        uid: u16,
        gid: u16,
    ) -> QfsInodeRecord {
        let mut record = QfsInodeRecord {
            inode,
            object_id: inode,
            parent_inode,
            kind: encode_inode_kind(kind),
            name_len: name.len() as u8,
            metadata_flags: QFS_OBJECT_SERVICE_AWARE,
            mode,
            uid,
            gid,
            links: 1,
            name: [0; QFS_NAME_BYTES],
            ..QfsInodeRecord::empty()
        };
        record.name[..name.len()].copy_from_slice(name.as_bytes());
        record.refresh_path_identity();
        record
    }

    fn filesystem_with_device(
        device: &'static dyn BlockStorageDevice,
        superblock: QfsSuperblock,
    ) -> QfsFileSystem {
        let fs = QfsFileSystem::new_on_block_device(false, device);
        {
            let mut state = fs.state.lock();
            state.mounted = true;
            state.superblock = superblock;
        }
        fs
    }

    fn format_device_with_inode(
        device: &dyn BlockStorageDevice,
        superblock: QfsSuperblock,
        inode: QfsInodeRecord,
    ) {
        let mut sector = [0u8; QFS_SECTOR_SIZE];
        superblock.write_sector(&mut sector).unwrap();
        device
            .write_sectors(QFS_SUPERBLOCK_SECTOR, &sector)
            .unwrap();

        let mut header = QfsBookHeader::empty();
        header.book_id = 0;
        header.chapter_count = 2;
        header.index_entry_count = 2;
        header.write_sector(&mut sector).unwrap();
        device
            .write_sectors(book_start_sector(&superblock, 0), &sector)
            .unwrap();

        sector.fill(0);
        QfsBookIndexEntry {
            chapter_id: 1,
            first_page: 1,
            page_count: 1,
            role: QfsBookRole::Inode,
            flags: 0,
            reserved: [0; 6],
        }
        .write(&mut sector[0..QFS_BOOK_INDEX_ENTRY_BYTES])
        .unwrap();
        QfsBookIndexEntry {
            chapter_id: 2,
            first_page: 3,
            page_count: 2,
            role: QfsBookRole::Data,
            flags: 0,
            reserved: [0; 6],
        }
        .write(&mut sector[QFS_BOOK_INDEX_ENTRY_BYTES..QFS_BOOK_INDEX_ENTRY_BYTES * 2])
        .unwrap();
        device
            .write_sectors(
                book_start_sector(&superblock, 0) + QFS_BOOK_HEADER_SECTORS,
                &sector,
            )
            .unwrap();

        let mut inodes = [None; QFS_MAX_INODE_RECORDS];
        inodes[0] = Some(QfsInodeRecord::root());
        inodes[1] = Some(inode);
        serialize_inode_records(&inodes, &mut sector).unwrap();
        device
            .write_sectors(
                page_location_sector(&superblock, superblock.inode_table).unwrap(),
                &sector,
            )
            .unwrap();
        device.flush().unwrap();
    }

    fn add_data_mapping(fs: &QfsFileSystem, inode: InodeId) {
        let mut state = fs.state.lock();
        state.chapter_index[0] = Some(QfsChapterIndexEntry {
            inode: inode.raw(),
            logical_page: 0,
            book_id: 0,
            first_page: 3,
            page_count: 2,
            flags: 0,
        });
        state.chapter_index[1] = Some(QfsChapterIndexEntry {
            inode: inode.raw(),
            logical_page: 2,
            book_id: 1,
            first_page: 3,
            page_count: 2,
            flags: 0,
        });
        if let Some(slot) = state.inode_slot_by_id(inode) {
            let mut record = state.inodes[slot].unwrap();
            record.first_chapter = 0;
            record.chapter_count = 2;
            state.inodes[slot] = Some(record);
        }
    }

    #[test]
    fn superblock_parse_serialize_round_trip() {
        let mut superblock = test_superblock(3);
        superblock.root_inode = 42;
        superblock.reserved[0] = 0xab;
        superblock.reserved[63] = 0xcd;

        let mut sector = [0u8; QFS_SECTOR_SIZE];
        superblock.write_sector(&mut sector).unwrap();
        let parsed = QfsSuperblock::parse_sector(&sector).unwrap();

        assert_eq!(parsed, superblock);
    }

    #[test]
    fn inode_object_metadata_round_trips_fixed_record() {
        let mut record = named_inode(
            9,
            InodeId::ROOT.raw(),
            InodeKind::RegularFile,
            "signed",
            0o640,
            7,
            8,
        );
        record.metadata_flags =
            QFS_OBJECT_HAS_SIGNATURE | QFS_OBJECT_HAS_CAPABILITY | QFS_OBJECT_SERVICE_AWARE;
        record.service_class = 3;
        record.extent_map_version = 4;
        record.extent_count = 1;
        record.extents[0] = QfsExtentRecord {
            logical_page: 5,
            book_id: 2,
            first_page: 11,
            page_count: 6,
        };
        record.signature_len = 16;
        record.signature_ref = [0xa5; 16];
        record.capability_len = 16;
        record.capability_ref = [0x5a; 16];
        record.last_transaction_id = 77;
        record.mutation_state = QfsObjectMutationState::Committed as u16;

        let mut inodes = [None; QFS_MAX_INODE_RECORDS];
        inodes[0] = Some(record);
        let mut sector = [0u8; QFS_SECTOR_SIZE];
        serialize_inode_records(&inodes, &mut sector).unwrap();

        let mut parsed = [None; QFS_MAX_INODE_RECORDS];
        parse_inode_records(&sector, &mut parsed).unwrap();
        assert_eq!(parsed[0], Some(record));
        assert_eq!(parsed[1], None);
    }

    #[test]
    fn book_header_round_trip_and_chapter_index_lookup() {
        let mut header = QfsBookHeader::empty();
        header.book_id = 7;
        header.chapter_count = 3;
        header.index_entry_count = 1;
        header.checksum = 0xdead_beef;
        header.generation = 9;
        header.reserved[0] = 0x5a;

        let mut sector = [0u8; QFS_SECTOR_SIZE];
        header.write_sector(&mut sector).unwrap();
        assert_eq!(QfsBookHeader::parse_sector(&sector).unwrap(), header);

        let entry = QfsBookIndexEntry {
            chapter_id: 77,
            first_page: 4,
            page_count: 6,
            role: QfsBookRole::Data,
            flags: 0x80,
            reserved: [1, 2, 3, 4, 5, 6],
        };
        sector.fill(0);
        entry
            .write(&mut sector[..QFS_BOOK_INDEX_ENTRY_BYTES])
            .unwrap();
        let mut parsed_entries = [None; QFS_MAX_BOOK_INDEX_ENTRIES];
        parse_book_index_entries(&sector, 1, &mut parsed_entries).unwrap();
        assert_eq!(parsed_entries[0], Some(entry));

        let fs = QfsFileSystem::new(false);
        let inode = InodeId::new(12);
        {
            let mut state = fs.state.lock();
            state.superblock = test_superblock(2);
            state.inodes[1] = Some(named_inode(
                inode.raw(),
                InodeId::ROOT.raw(),
                InodeKind::RegularFile,
                "data",
                0o644,
                0,
                0,
            ));
            state.chapter_index[0] = Some(QfsChapterIndexEntry {
                inode: inode.raw(),
                logical_page: 2,
                book_id: 1,
                first_page: 5,
                page_count: 3,
                flags: 0,
            });
        }
        let state = fs.state.lock();
        let record = state.inode_by_id(inode).unwrap();
        assert_eq!(
            QfsFileSystem::data_sector_for_offset(
                &state,
                record,
                QFS_PAGE_SECTORS as u64 * QFS_SECTOR_SIZE as u64 * 2
            ),
            Some((
                book_start_sector(&state.superblock, 1) + 5 * QFS_PAGE_SECTORS as u64,
                0
            ))
        );
    }

    #[test]
    fn file_offset_translates_to_book_chapter_page_and_sector() {
        let fs = QfsFileSystem::new(false);
        let inode = InodeId::new(20);
        {
            let mut state = fs.state.lock();
            state.superblock = QfsSuperblock {
                total_books: 3,
                page_sectors: 2,
                book_pages: 8,
                ..QfsSuperblock::empty()
            };
            state.inodes[1] = Some(named_inode(
                inode.raw(),
                InodeId::ROOT.raw(),
                InodeKind::RegularFile,
                "mapped",
                0o644,
                0,
                0,
            ));
            state.chapter_index[3] = Some(QfsChapterIndexEntry {
                inode: inode.raw(),
                logical_page: 4,
                book_id: 2,
                first_page: 6,
                page_count: 2,
                flags: 0,
            });
            let mut record = state.inodes[1].unwrap();
            record.first_chapter = 3;
            record.chapter_count = 1;
            state.inodes[1] = Some(record);
        }

        let state = fs.state.lock();
        let record = state.inode_by_id(inode).unwrap();
        let offset = 5 * (2 * QFS_SECTOR_SIZE) as u64 + QFS_SECTOR_SIZE as u64 + 17;
        let expected_sector = book_start_sector(&state.superblock, 2) + 7 * 2 + 1;
        assert_eq!(
            QfsFileSystem::data_sector_for_offset(&state, record, offset),
            Some((expected_sector, 17))
        );
    }

    #[test]
    fn inode_allocation_and_lookup() {
        let fs = QfsFileSystem::new(false);
        fs.mkdir(
            path("/docs"),
            Permissions::new(0o755, 0, 0),
            Credentials::kernel(),
        )
        .unwrap();
        fs.symlink("/docs", path("/docs-link"), Credentials::kernel())
            .unwrap();

        let docs = fs.lookup(path("/docs")).unwrap();
        let link = fs.lookup(path("/docs-link")).unwrap();

        assert_eq!(docs.kind, InodeKind::Directory);
        assert_eq!(link.kind, InodeKind::Symlink);
        assert_eq!(fs.lookup_inode(docs.id).unwrap().id, docs.id);
        assert!(docs.id.raw() > InodeId::ROOT.raw());
        assert!(link.id.raw() > docs.id.raw());
    }

    #[test]
    fn directory_create_remove_and_rename_operations() {
        let fs = QfsFileSystem::new(false);
        fs.mkdir(
            path("/alpha"),
            Permissions::new(0o755, 0, 0),
            Credentials::kernel(),
        )
        .unwrap();
        fs.rename(path("/alpha"), path("/beta"), Credentials::kernel())
            .unwrap();

        assert_eq!(fs.lookup(path("/alpha")), Err(FsError::NotFound));
        assert_eq!(fs.lookup(path("/beta")).unwrap().kind, InodeKind::Directory);

        fs.rmdir(path("/beta"), Credentials::kernel()).unwrap();
        assert_eq!(fs.lookup(path("/beta")), Err(FsError::NotFound));
    }

    #[test]
    fn journal_replay_ignores_begin_without_commit() {
        let device = Box::leak(Box::new(MemBlockDevice::new(256)));
        let superblock = test_superblock(1);
        let target_sector = 20;
        let original = [0x11u8; QFS_SECTOR_SIZE];
        let replacement = [0x22u8; QFS_SECTOR_SIZE];
        device.write_sector_copy(target_sector, &original);

        let fs = filesystem_with_device(device, superblock);
        let tx = fs.begin_transaction().unwrap();
        fs.journal_write(
            tx,
            QfsJournalRecordKind::MetadataWrite,
            0,
            target_sector,
            &replacement,
        )
        .unwrap();

        let replay = filesystem_with_device(device, superblock);
        replay.replay_journal().unwrap();

        assert_eq!(device.read_sector_copy(target_sector), original);
    }

    #[test]
    fn journal_replay_completes_committed_transaction_with_incomplete_home_writes() {
        let device = Box::leak(Box::new(MemBlockDevice::new(256)));
        let superblock = test_superblock(1);
        let target_sector = 21;
        let partial = [0x33u8; QFS_SECTOR_SIZE];
        let committed = [0x44u8; QFS_SECTOR_SIZE];
        device.write_sector_copy(target_sector, &partial);

        let fs = filesystem_with_device(device, superblock);
        let tx = fs.begin_transaction().unwrap();
        fs.journal_write(
            tx,
            QfsJournalRecordKind::MetadataWrite,
            0,
            target_sector,
            &committed,
        )
        .unwrap();
        fs.append_journal_record(
            QfsJournalRecord {
                sequence: tx,
                record_type: QfsJournalRecordKind::Commit as u16,
                target_inode: 0,
                sector: 0,
                sector_count: 0,
                checksum: 0,
                flags: 0,
            },
            None,
        )
        .unwrap();

        let replay = filesystem_with_device(device, superblock);
        replay.replay_journal().unwrap();

        assert_eq!(device.read_sector_copy(target_sector), committed);
    }

    #[test]
    fn pread_pwrite_cross_page_and_book_boundaries() {
        let device = Box::leak(Box::new(MemBlockDevice::new(256)));
        let inode = InodeId::new(30);
        let superblock = QfsSuperblock {
            total_books: 2,
            page_sectors: 1,
            book_pages: 32,
            inode_table: QfsPageLocation::new(0, 1),
            journal: QfsPageLocation::new(0, 8),
            total_sectors: 65,
            ..QfsSuperblock::empty()
        };
        let fs = filesystem_with_device(device, superblock);
        {
            let mut state = fs.state.lock();
            state.inodes[1] = Some(named_inode(
                inode.raw(),
                InodeId::ROOT.raw(),
                InodeKind::RegularFile,
                "big",
                0o666,
                0,
                0,
            ));
        }
        add_data_mapping(&fs, inode);
        let file = File::new(inode, FileMode::ReadWrite);

        let page_boundary_offset = QFS_SECTOR_SIZE as u64 - 5;
        let page_data = *b"page-boundary-write";
        assert_eq!(
            fs.pwrite(&file, &page_data, page_boundary_offset).unwrap(),
            page_data.len()
        );
        let mut page_read = [0u8; 19];
        assert_eq!(
            fs.pread(&file, &mut page_read, page_boundary_offset)
                .unwrap(),
            page_read.len()
        );
        assert_eq!(page_read, page_data);

        let book_boundary_offset = QFS_SECTOR_SIZE as u64 * 2 - 7;
        let book_data = *b"book-boundary-write";
        assert_eq!(
            fs.pwrite(&file, &book_data, book_boundary_offset).unwrap(),
            book_data.len()
        );
        let mut book_read = [0u8; 19];
        assert_eq!(
            fs.pread(&file, &mut book_read, book_boundary_offset)
                .unwrap(),
            book_read.len()
        );
        assert_eq!(book_read, book_data);
    }

    #[test]
    fn permission_checks_use_permissions_and_credentials() {
        let fs = QfsFileSystem::new(false);
        let owner = Credentials::user(1000, 1000);
        let stranger = Credentials::user(2000, 2000);
        let group_member = Credentials::user(2001, 1000);
        let perms = Permissions::new(0o640, owner.uid, owner.gid);

        assert!(perms.allows(owner, AccessMode::ReadWrite));
        assert!(perms.allows(group_member, AccessMode::Read));
        assert!(!perms.allows(group_member, AccessMode::Write));
        assert!(!perms.allows(stranger, AccessMode::Read));
        assert!(perms.allows(Credentials::kernel(), AccessMode::ReadWrite));

        fs.chmod(path("/"), 0o555, Credentials::kernel()).unwrap();
        assert_eq!(
            fs.mkdir(
                path("/blocked"),
                Permissions::new(0o755, 2000, 2000),
                stranger
            ),
            Err(FsError::PermissionDenied)
        );
        fs.mkdir(
            path("/allowed"),
            Permissions::new(0o755, 0, 0),
            Credentials::kernel(),
        )
        .unwrap();
    }

    #[cfg(feature = "qfs-std")]
    #[test]
    fn host_backed_qfs_image_persists_inline_file_data_across_remount() {
        use std::fs::OpenOptions;
        use std::path::PathBuf;

        use crate::kernel::fs::qfs_std::StdQfsBlockDevice;

        fn image_path() -> PathBuf {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "mirage_qfs_mount_{}_{}.img",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            path
        }

        let image = image_path();
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(&image)
            .unwrap();
        let device = Box::leak(Box::new(
            StdQfsBlockDevice::create_sized(file, QFS_SECTOR_SIZE, 256).unwrap(),
        ));
        let file_inode = named_inode(
            2,
            InodeId::ROOT.raw(),
            InodeKind::RegularFile,
            "persist",
            0o666,
            0,
            0,
        );
        let superblock = test_superblock(1);
        format_device_with_inode(device, superblock, file_inode);

        let fs = QfsFileSystem::new_on_block_device(false, device);
        fs.refresh_from_block_device().unwrap();
        let metadata = fs.lookup(path("/persist")).unwrap();
        let file = File::new(metadata.id, FileMode::ReadWrite);
        let payload = b"persistent qfs inline payload";
        assert_eq!(fs.pwrite(&file, payload, 0).unwrap(), payload.len());

        let remounted = QfsFileSystem::new_on_block_device(false, device);
        remounted.refresh_from_block_device().unwrap();
        let metadata = remounted.lookup(path("/persist")).unwrap();
        let file = File::new(metadata.id, FileMode::ReadOnly);
        let mut read_back = [0u8; 29];
        assert_eq!(
            remounted.pread(&file, &mut read_back, 0).unwrap(),
            payload.len()
        );
        assert_eq!(&read_back[..payload.len()], payload);

        std::fs::remove_file(image).ok();
    }
}
