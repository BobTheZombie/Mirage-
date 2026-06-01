//! Inode identifiers and metadata shared by filesystem implementations.

use crate::kernel::fs::permissions::Permissions;

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
    BlockDevice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InodeMetadata {
    pub id: InodeId,
    pub kind: InodeKind,
    pub size: u64,
    pub permissions: Permissions,
}

impl InodeMetadata {
    pub const fn new(id: InodeId, kind: InodeKind, size: u64, permissions: Permissions) -> Self {
        Self {
            id,
            kind,
            size,
            permissions,
        }
    }
}
