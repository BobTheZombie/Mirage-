//! Heap-free virtual filesystem scaffolding for Mirage kernel storage.
//!
//! The module mirrors the fixed-table style used by the rest of `src/kernel`:
//! paths are borrowed, mount state is stored in const-generic arrays, and the
//! SSD/USB implementation uses bounded inline node/data storage while syncing
//! metadata and file contents through a sector-addressed block-device trait.

pub mod ext4;
pub mod file;
pub mod inode;
pub mod mount;
pub mod path;
pub mod permissions;
pub mod qfs;
#[cfg(feature = "qfs-std")]
pub mod qfs_std;
pub mod ssd_usb;
pub mod stdlib;
pub mod vfs;

pub use ext4::{Ext4Backend, Ext4Error, Ext4Superblock, SsdUsbOptions};
pub use file::{
    DescriptorFlags, File, FileDescriptionId, FileHandle, FileMode, FileTable, FileTableError,
    OpenFileDescription, OpenFlags,
};
pub use inode::{Dentry, DirEntry, Inode, InodeId, InodeKind, InodeMetadata, Stat};
pub use mount::{Mount, MountError, MountTable};
pub use path::{Path, PathError, MAX_COMPONENT_BYTES, MAX_PATH_BYTES};
pub use permissions::{AccessMode, Credentials as FsCredentials, Permissions};
#[cfg(feature = "qfs-std")]
pub use qfs_std::StdQfsBlockDevice;

pub use qfs::{
    QfsBookHeader, QfsBookIndexEntry, QfsBookRole, QfsChapterIndexEntry, QfsFileSystem,
    QfsInodeRecord, QfsJournalRecord, QfsJournalRecordKind, QfsPageLocation, QfsSuperblock,
    QfsTransactionId, QFS_BOOK_INDEX_SECTORS, QFS_BOOK_PAGES, QFS_INLINE_DATA_BYTES, QFS_MAGIC,
    QFS_MAX_BOOKS, QFS_MAX_BOOK_INDEX_ENTRIES, QFS_MAX_CHAPTER_INDEX_ENTRIES,
    QFS_MAX_INODE_RECORDS, QFS_MAX_JOURNAL_RECORDS, QFS_NAME_BYTES, QFS_PAGE_SECTORS,
    QFS_SECTOR_SIZE, QFS_VERSION,
};
pub use ssd_usb::{SsdUsbFileSystem, MAX_FILE_BYTES, MAX_NAME_BYTES, MAX_VOLUME_NODES};
pub use stdlib::{
    errno_from_vfs, negative_errno_from_vfs, open_flags_from_libc, permissions_from_libc_mode,
    syscall_error_code_from_vfs, CDirEntry, CStat, DT_BLK, DT_CHR, DT_DIR, DT_FIFO, DT_LNK, DT_REG,
    DT_SOCK, DT_UNKNOWN, F_OK, O_APPEND, O_CLOEXEC, O_CREAT, O_DIRECTORY, O_EXCL, O_NOFOLLOW,
    O_RDONLY, O_RDWR, O_TRUNC, O_WRONLY, R_OK, S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK, S_IFMT,
    S_IFREG, S_IFSOCK, W_OK, X_OK,
};
pub use vfs::{FileSystem, FsError, SuperBlock, VfsError};
