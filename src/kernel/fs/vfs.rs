//! VFS traits and shared errors for heap-free filesystem backends.

use crate::kernel::fs::{
    file::{FileHandle, FileMode},
    inode::{InodeId, InodeMetadata},
    path::{Path, PathError},
    permissions::{AccessMode, Credentials},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    InvalidPath(PathError),
    NotFound,
    NotDirectory,
    AlreadyExists,
    PermissionDenied,
    ReadOnly,
    NoSpace,
    InvalidHandle,
    Unsupported,
}

impl From<PathError> for FsError {
    fn from(value: PathError) -> Self {
        Self::InvalidPath(value)
    }
}

pub trait FileSystem {
    fn root_inode(&self) -> InodeId;
    fn lookup(&self, path: Path<'_>) -> Result<InodeMetadata, FsError>;
    fn open(
        &self,
        path: Path<'_>,
        mode: FileMode,
        credentials: Credentials,
    ) -> Result<FileHandle, FsError> {
        let metadata = self.lookup(path)?;
        let access = match mode {
            FileMode::ReadOnly => AccessMode::Read,
            FileMode::WriteOnly => AccessMode::Write,
            FileMode::ReadWrite => AccessMode::ReadWrite,
        };
        if !metadata.permissions.allows(credentials, access) {
            return Err(FsError::PermissionDenied);
        }
        Ok(FileHandle::new(metadata.id, mode))
    }
    fn read(&self, handle: &mut FileHandle, buffer: &mut [u8]) -> Result<usize, FsError>;
    fn write(&self, handle: &mut FileHandle, data: &[u8]) -> Result<usize, FsError>;
}
