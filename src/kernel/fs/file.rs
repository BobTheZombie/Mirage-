//! Open file descriptions, flags, and fixed-capacity file registries.

use crate::kernel::fs::inode::InodeId;

/// POSIX-style access mode for an open file description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl FileMode {
    pub const fn can_read(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    pub const fn can_write(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }
}

/// Heap-free bitflags used when opening files through the VFS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenFlags(u32);

impl OpenFlags {
    pub const EMPTY: Self = Self(0);
    pub const RDONLY: Self = Self(1 << 0);
    pub const WRONLY: Self = Self(1 << 1);
    pub const RDWR: Self = Self(1 << 2);
    pub const CREATE: Self = Self(1 << 3);
    pub const EXCLUSIVE: Self = Self(1 << 4);
    pub const TRUNCATE: Self = Self(1 << 5);
    pub const APPEND: Self = Self(1 << 6);
    pub const DIRECTORY: Self = Self(1 << 7);
    pub const NOFOLLOW: Self = Self(1 << 8);

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) == flag.0
    }

    pub const fn union(self, flag: Self) -> Self {
        Self(self.0 | flag.0)
    }

    pub const fn access_mode(self) -> FileMode {
        if self.contains(Self::RDWR) {
            FileMode::ReadWrite
        } else if self.contains(Self::WRONLY) {
            FileMode::WriteOnly
        } else {
            FileMode::ReadOnly
        }
    }
}

/// Kernel-facing open file description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct File {
    inode: InodeId,
    cursor: u64,
    mode: FileMode,
    flags: OpenFlags,
}

impl File {
    pub const fn new(inode: InodeId, mode: FileMode) -> Self {
        Self {
            inode,
            cursor: 0,
            mode,
            flags: OpenFlags::EMPTY,
        }
    }

    pub const fn with_flags(inode: InodeId, flags: OpenFlags) -> Self {
        Self {
            inode,
            cursor: 0,
            mode: flags.access_mode(),
            flags,
        }
    }

    pub const fn inode(self) -> InodeId {
        self.inode
    }

    pub const fn cursor(self) -> u64 {
        self.cursor
    }

    pub const fn mode(self) -> FileMode {
        self.mode
    }

    pub const fn flags(self) -> OpenFlags {
        self.flags
    }

    pub const fn is_append(self) -> bool {
        self.flags.contains(OpenFlags::APPEND)
    }

    pub fn seek(&mut self, offset: u64) {
        self.cursor = offset;
    }

    pub fn advance(&mut self, bytes: usize) {
        self.cursor = self.cursor.saturating_add(bytes as u64);
    }
}

/// Backward-compatible name used by early Mirage filesystem code.
pub type FileHandle = File;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileTableError {
    Full,
    InvalidDescriptor,
}

/// Fixed-capacity open-file table suitable for kernel tasks without allocation.
pub struct FileTable<const MAX: usize> {
    files: [Option<File>; MAX],
}

impl<const MAX: usize> FileTable<MAX> {
    pub const fn new() -> Self {
        Self { files: [None; MAX] }
    }

    pub fn insert(&mut self, file: File) -> Result<usize, FileTableError> {
        let mut idx = 0usize;
        while idx < MAX {
            if self.files[idx].is_none() {
                self.files[idx] = Some(file);
                return Ok(idx);
            }
            idx += 1;
        }
        Err(FileTableError::Full)
    }

    pub fn get(&self, fd: usize) -> Result<File, FileTableError> {
        self.files
            .get(fd)
            .and_then(|entry| *entry)
            .ok_or(FileTableError::InvalidDescriptor)
    }

    pub fn get_mut(&mut self, fd: usize) -> Result<&mut File, FileTableError> {
        self.files
            .get_mut(fd)
            .and_then(Option::as_mut)
            .ok_or(FileTableError::InvalidDescriptor)
    }

    pub fn close(&mut self, fd: usize) -> Result<File, FileTableError> {
        let slot = self
            .files
            .get_mut(fd)
            .ok_or(FileTableError::InvalidDescriptor)?;
        slot.take().ok_or(FileTableError::InvalidDescriptor)
    }
}
