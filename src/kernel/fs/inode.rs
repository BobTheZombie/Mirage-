//! Inode identifiers, metadata, directory entries, and inode operations.

use crate::kernel::fs::{path::MAX_COMPONENT_BYTES, permissions::Permissions, vfs::VfsError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InodeId(pub u64);

impl InodeId {
    pub const ROOT: Self = Self(1);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InodeKind {
    Directory,
    RegularFile,
    Symlink,
    BlockDevice,
    CharDevice,
    Fifo,
    Socket,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InodeMetadata {
    pub id: InodeId,
    pub kind: InodeKind,
    pub size: u64,
    pub permissions: Permissions,
    pub links: u16,
}

impl InodeMetadata {
    pub const fn new(id: InodeId, kind: InodeKind, size: u64, permissions: Permissions) -> Self {
        Self {
            id,
            kind,
            size,
            permissions,
            links: 1,
        }
    }

    pub const fn with_links(
        id: InodeId,
        kind: InodeKind,
        size: u64,
        permissions: Permissions,
        links: u16,
    ) -> Self {
        Self {
            id,
            kind,
            size,
            permissions,
            links,
        }
    }
}

/// POSIX-like stat payload returned by VFS metadata calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub inode: InodeId,
    pub kind: InodeKind,
    pub size: u64,
    pub mode: u16,
    pub uid: u16,
    pub gid: u16,
    pub links: u16,
}

impl Stat {
    pub const fn from_metadata(metadata: InodeMetadata) -> Self {
        Self {
            inode: metadata.id,
            kind: metadata.kind,
            size: metadata.size,
            mode: metadata.permissions.bits(),
            uid: metadata.permissions.owner(),
            gid: metadata.permissions.group(),
            links: metadata.links,
        }
    }
}

/// Fixed-size directory entry returned during directory iteration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub inode: InodeId,
    pub kind: InodeKind,
    name: [u8; MAX_COMPONENT_BYTES],
    name_len: usize,
}

impl DirEntry {
    pub const fn empty() -> Self {
        Self {
            inode: InodeId::new(0),
            kind: InodeKind::RegularFile,
            name: [0; MAX_COMPONENT_BYTES],
            name_len: 0,
        }
    }

    pub fn new(inode: InodeId, kind: InodeKind, name: &str) -> Result<Self, VfsError> {
        if name.len() > MAX_COMPONENT_BYTES {
            return Err(VfsError::NameTooLong);
        }
        let mut entry = Self::empty();
        entry.inode = inode;
        entry.kind = kind;
        entry.name_len = name.len();
        entry.name[..name.len()].copy_from_slice(name.as_bytes());
        Ok(entry)
    }

    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }
}

/// Cached directory entry linking a name to an inode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dentry {
    pub parent: InodeId,
    pub inode: InodeId,
    pub kind: InodeKind,
    name: [u8; MAX_COMPONENT_BYTES],
    name_len: usize,
}

impl Dentry {
    pub fn new(
        parent: InodeId,
        inode: InodeId,
        kind: InodeKind,
        name: &str,
    ) -> Result<Self, VfsError> {
        if name.len() > MAX_COMPONENT_BYTES {
            return Err(VfsError::NameTooLong);
        }
        let mut dentry = Self {
            parent,
            inode,
            kind,
            name: [0; MAX_COMPONENT_BYTES],
            name_len: name.len(),
        };
        dentry.name[..name.len()].copy_from_slice(name.as_bytes());
        Ok(dentry)
    }

    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }
}

/// Kernel-facing inode operation table implemented by concrete filesystems.
pub trait Inode {
    fn id(&self) -> InodeId;
    fn metadata(&self) -> InodeMetadata;
    fn read_at(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize, VfsError> {
        Err(VfsError::Unsupported)
    }
    fn write_at(&self, _offset: u64, _data: &[u8]) -> Result<usize, VfsError> {
        Err(VfsError::Unsupported)
    }
    fn iterate_dir(&self, _offset: usize, _entries: &mut [DirEntry]) -> Result<usize, VfsError> {
        Err(VfsError::Unsupported)
    }
}
