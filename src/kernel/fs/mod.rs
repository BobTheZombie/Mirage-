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
pub mod ssd_usb;
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
pub use ssd_usb::{SsdUsbFileSystem, MAX_FILE_BYTES, MAX_NAME_BYTES, MAX_VOLUME_NODES};
pub use vfs::{FileSystem, FsError, SuperBlock, VfsError};
