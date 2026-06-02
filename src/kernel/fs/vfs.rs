//! VFS traits, POSIX-like operation surface, and shared errors.

use crate::kernel::fs::{
    file::{File, FileMode, OpenFlags},
    inode::{DirEntry, InodeId, InodeKind, InodeMetadata, Stat},
    path::{Path, PathError},
    permissions::{AccessMode, Credentials, Permissions},
};

/// Errors surfaced by kernel-facing VFS operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VfsError {
    InvalidPath(PathError),
    NameTooLong,
    NotFound,
    NotDirectory,
    IsDirectory,
    AlreadyExists,
    PermissionDenied,
    ReadOnly,
    NoSpace,
    InvalidHandle,
    InvalidArgument,
    Busy,
    CrossDevice,
    TooManyLinks,
    Unsupported,
}

impl From<PathError> for VfsError {
    fn from(value: PathError) -> Self {
        Self::InvalidPath(value)
    }
}

impl VfsError {
    /// Linux errno-compatible numeric value for this VFS failure.
    ///
    /// Mirage keeps structured internal errors, but syscall/libc boundaries use
    /// these errno assignments so applications can reason about failures with
    /// POSIX conventions. Path syntax failures that do not have a direct Linux
    /// equivalent are reported as `EINVAL`.
    pub const fn linux_errno(self) -> i32 {
        match self {
            VfsError::InvalidPath(PathError::TooLong)
            | VfsError::InvalidPath(PathError::ComponentTooLong)
            | VfsError::NameTooLong => 36,
            VfsError::InvalidPath(PathError::Empty) | VfsError::NotFound => 2,
            VfsError::InvalidPath(PathError::NotAbsolute)
            | VfsError::InvalidPath(PathError::InvalidByte)
            | VfsError::InvalidArgument => 22,
            VfsError::NotDirectory => 20,
            VfsError::IsDirectory => 21,
            VfsError::AlreadyExists => 17,
            VfsError::PermissionDenied => 13,
            VfsError::ReadOnly => 30,
            VfsError::NoSpace => 28,
            VfsError::InvalidHandle => 9,
            VfsError::Busy => 16,
            VfsError::CrossDevice => 18,
            VfsError::TooManyLinks => 31,
            VfsError::Unsupported => 95,
        }
    }
}

/// Backward-compatible error name used by early filesystem code.
pub type FsError = VfsError;

/// Superblock state common to mounted filesystems.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuperBlock {
    pub device: u64,
    pub block_size: u32,
    pub total_blocks: u64,
    pub free_blocks: u64,
    pub root: InodeId,
    pub read_only: bool,
}

impl SuperBlock {
    pub const fn new(root: InodeId) -> Self {
        Self {
            device: 0,
            block_size: 1,
            total_blocks: 0,
            free_blocks: 0,
            root,
            read_only: false,
        }
    }
}

/// Kernel-facing filesystem operations mirroring Linux/POSIX basics.
pub trait FileSystem {
    fn root_inode(&self) -> InodeId;

    fn super_block(&self) -> SuperBlock {
        SuperBlock::new(self.root_inode())
    }

    fn lookup(&self, path: Path<'_>) -> Result<InodeMetadata, VfsError>;

    fn open(
        &self,
        path: Path<'_>,
        flags: OpenFlags,
        credentials: Credentials,
    ) -> Result<File, VfsError> {
        let metadata = self.lookup(path)?;
        if flags.contains(OpenFlags::DIRECTORY) && metadata.kind != InodeKind::Directory {
            return Err(VfsError::NotDirectory);
        }
        if metadata.kind == InodeKind::Directory && flags.access_mode().can_write() {
            return Err(VfsError::IsDirectory);
        }
        let access = access_for_mode(flags.access_mode());
        if !metadata.permissions.allows(credentials, access) {
            return Err(VfsError::PermissionDenied);
        }
        Ok(File::with_flags(metadata.id, flags).with_path(path))
    }

    fn close(&self, _file: File) -> Result<(), VfsError> {
        Ok(())
    }

    fn read(&self, file: &mut File, buffer: &mut [u8]) -> Result<usize, VfsError> {
        if !file.mode().can_read() {
            return Err(VfsError::PermissionDenied);
        }
        let bytes = self.pread(file, buffer, file.cursor())?;
        file.advance(bytes);
        Ok(bytes)
    }

    fn write(&self, file: &mut File, data: &[u8]) -> Result<usize, VfsError> {
        if !file.mode().can_write() {
            return Err(VfsError::PermissionDenied);
        }
        let bytes = self.pwrite(file, data, file.cursor())?;
        file.advance(bytes);
        Ok(bytes)
    }

    fn pread(&self, _file: &File, _buffer: &mut [u8], _offset: u64) -> Result<usize, VfsError> {
        Err(VfsError::Unsupported)
    }

    fn pwrite(&self, _file: &File, _data: &[u8], _offset: u64) -> Result<usize, VfsError> {
        Err(VfsError::Unsupported)
    }

    fn mkdir(
        &self,
        _path: Path<'_>,
        _mode: Permissions,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn rmdir(&self, _path: Path<'_>, _credentials: Credentials) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn unlink(&self, _path: Path<'_>, _credentials: Credentials) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn rename(
        &self,
        _old_path: Path<'_>,
        _new_path: Path<'_>,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn link(
        &self,
        _old_path: Path<'_>,
        _new_path: Path<'_>,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn symlink(
        &self,
        _target: &str,
        _link_path: Path<'_>,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn readlink(&self, _path: Path<'_>, _buffer: &mut [u8]) -> Result<usize, VfsError> {
        Err(VfsError::Unsupported)
    }

    fn stat(&self, path: Path<'_>) -> Result<Stat, VfsError> {
        Ok(Stat::from_metadata(self.lookup(path)?))
    }

    fn fstat(&self, file: &File) -> Result<Stat, VfsError> {
        Ok(Stat::from_metadata(self.lookup_inode(file.inode())?))
    }

    fn lookup_inode(&self, _inode: InodeId) -> Result<InodeMetadata, VfsError> {
        Err(VfsError::Unsupported)
    }

    fn chmod(
        &self,
        _path: Path<'_>,
        _mode: u16,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn chown(
        &self,
        _path: Path<'_>,
        _uid: u16,
        _gid: u16,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn truncate(
        &self,
        _path: Path<'_>,
        _size: u64,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn ftruncate(
        &self,
        _file: &File,
        _size: u64,
        _credentials: Credentials,
    ) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn fsync(&self, _file: &File) -> Result<(), VfsError> {
        Ok(())
    }

    fn readdir(
        &self,
        _path: Path<'_>,
        _offset: usize,
        _entries: &mut [DirEntry],
    ) -> Result<usize, VfsError> {
        Err(VfsError::Unsupported)
    }

    fn readdir_inode(
        &self,
        _inode: InodeId,
        _offset: usize,
        _entries: &mut [DirEntry],
    ) -> Result<usize, VfsError> {
        Err(VfsError::Unsupported)
    }
}

const fn access_for_mode(mode: FileMode) -> AccessMode {
    match mode {
        FileMode::ReadOnly => AccessMode::Read,
        FileMode::WriteOnly => AccessMode::Write,
        FileMode::ReadWrite => AccessMode::ReadWrite,
    }
}
