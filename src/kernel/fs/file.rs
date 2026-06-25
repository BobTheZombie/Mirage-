//! Open file descriptions, descriptor flags, and fixed-capacity registries.

use crate::kernel::fs::{
    inode::InodeId,
    path::{Path, MAX_PATH_BYTES},
};

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
    pub const CLOSE_ON_EXEC: Self = Self(1 << 9);

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

    pub const fn without_descriptor_flags(self) -> Self {
        Self(self.0 & !Self::CLOSE_ON_EXEC.0)
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

/// Per-descriptor flags. These are intentionally separate from open-file
/// status flags because they are properties of the descriptor table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DescriptorFlags(u32);

impl DescriptorFlags {
    pub const EMPTY: Self = Self(0);
    pub const CLOSE_ON_EXEC: Self = Self(1 << 0);

    pub const fn from_open_flags(flags: OpenFlags) -> Self {
        if flags.contains(OpenFlags::CLOSE_ON_EXEC) {
            Self::CLOSE_ON_EXEC
        } else {
            Self::EMPTY
        }
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

    pub const fn without(self, flag: Self) -> Self {
        Self(self.0 & !flag.0)
    }
}

/// Kernel-facing open file description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct File {
    inode: InodeId,
    cursor: u64,
    mode: FileMode,
    flags: OpenFlags,
    path: [u8; MAX_PATH_BYTES],
    path_len: usize,
}

impl File {
    pub const fn new(inode: InodeId, mode: FileMode) -> Self {
        let mut path = [0u8; MAX_PATH_BYTES];
        path[0] = b'/';
        Self {
            inode,
            cursor: 0,
            mode,
            flags: OpenFlags::EMPTY,
            path,
            path_len: 1,
        }
    }

    pub const fn with_flags(inode: InodeId, flags: OpenFlags) -> Self {
        let file_flags = flags.without_descriptor_flags();
        let mut path = [0u8; MAX_PATH_BYTES];
        path[0] = b'/';
        Self {
            inode,
            cursor: 0,
            mode: file_flags.access_mode(),
            flags: file_flags,
            path,
            path_len: 1,
        }
    }

    pub fn with_path(mut self, path: Path<'_>) -> Self {
        let raw = path.as_str().as_bytes();
        self.path = [0u8; MAX_PATH_BYTES];
        self.path[..raw.len()].copy_from_slice(raw);
        self.path_len = raw.len();
        self
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

    pub fn path(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.path[..self.path_len]) }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileDescriptionId(usize);

impl FileDescriptionId {
    pub const fn new(raw: usize) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipeId(usize);

impl PipeId {
    pub const fn new(raw: usize) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeDirection {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipeEndpoint {
    id: PipeId,
    direction: PipeDirection,
}

impl PipeEndpoint {
    pub const fn new(id: PipeId, direction: PipeDirection) -> Self {
        Self { id, direction }
    }

    pub const fn id(self) -> PipeId {
        self.id
    }

    pub const fn direction(self) -> PipeDirection {
        self.direction
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventFdId(usize);

impl EventFdId {
    pub const fn new(raw: usize) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceHandle {
    id: crate::kernel::device::DeviceId,
}

impl DeviceHandle {
    pub const fn new(id: crate::kernel::device::DeviceId) -> Self {
        Self { id }
    }

    pub const fn id(self) -> crate::kernel::device::DeviceId {
        self.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SocketHandle {
    raw: u64,
}

impl SocketHandle {
    pub const fn new(raw: u64) -> Self {
        Self { raw }
    }

    pub const fn raw(self) -> u64 {
        self.raw
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleDescriptor {
    Stdout,
    Stderr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorObject {
    Regular(File),
    Pipe(PipeEndpoint),
    EventFd(EventFdId),
    Device(DeviceHandle),
    Socket(SocketHandle),
    Console(ConsoleDescriptor),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenFileDescription {
    object: DescriptorObject,
    ref_count: u16,
}

impl OpenFileDescription {
    pub const fn new(file: File) -> Self {
        Self::new_object(DescriptorObject::Regular(file))
    }

    pub const fn new_object(object: DescriptorObject) -> Self {
        Self {
            object,
            ref_count: 1,
        }
    }

    pub const fn file(self) -> Option<File> {
        match self.object {
            DescriptorObject::Regular(file) => Some(file),
            _ => None,
        }
    }

    pub const fn object(self) -> DescriptorObject {
        self.object
    }

    pub const fn ref_count(self) -> u16 {
        self.ref_count
    }

    fn increment_ref_count(&mut self) {
        self.ref_count = self.ref_count.saturating_add(1);
    }

    fn decrement_ref_count(&mut self) -> u16 {
        if self.ref_count > 0 {
            self.ref_count -= 1;
        }
        self.ref_count
    }
}

/// Fixed-capacity open-file-description table suitable for kernel tasks
/// without allocation. Descriptor tables in processes hold references to these
/// descriptions, allowing dup/fork-style sharing of cursor and status flags.
pub struct FileTable<const MAX: usize> {
    descriptions: [Option<OpenFileDescription>; MAX],
}

impl<const MAX: usize> FileTable<MAX> {
    pub const fn new() -> Self {
        Self {
            descriptions: [None; MAX],
        }
    }

    pub fn clear(&mut self) {
        let mut idx = 0usize;
        while idx < MAX {
            self.descriptions[idx] = None;
            idx += 1;
        }
    }

    pub fn insert(&mut self, file: File) -> Result<FileDescriptionId, FileTableError> {
        self.insert_object(DescriptorObject::Regular(file))
    }

    pub fn insert_object(
        &mut self,
        object: DescriptorObject,
    ) -> Result<FileDescriptionId, FileTableError> {
        let mut idx = 0usize;
        while idx < MAX {
            if self.descriptions[idx].is_none() {
                self.descriptions[idx] = Some(OpenFileDescription::new_object(object));
                return Ok(FileDescriptionId::new(idx));
            }
            idx += 1;
        }
        Err(FileTableError::Full)
    }

    pub fn get(&self, id: FileDescriptionId) -> Result<File, FileTableError> {
        self.descriptions
            .get(id.raw())
            .and_then(|entry| *entry)
            .and_then(OpenFileDescription::file)
            .ok_or(FileTableError::InvalidDescriptor)
    }

    pub fn get_object(&self, id: FileDescriptionId) -> Result<DescriptorObject, FileTableError> {
        self.descriptions
            .get(id.raw())
            .and_then(|entry| *entry)
            .map(OpenFileDescription::object)
            .ok_or(FileTableError::InvalidDescriptor)
    }

    pub fn get_mut(&mut self, id: FileDescriptionId) -> Result<&mut File, FileTableError> {
        self.descriptions
            .get_mut(id.raw())
            .and_then(Option::as_mut)
            .and_then(|description| match &mut description.object {
                DescriptorObject::Regular(file) => Some(file),
                _ => None,
            })
            .ok_or(FileTableError::InvalidDescriptor)
    }

    pub fn ref_count(&self, id: FileDescriptionId) -> Result<u16, FileTableError> {
        self.descriptions
            .get(id.raw())
            .and_then(|entry| *entry)
            .map(|description| description.ref_count())
            .ok_or(FileTableError::InvalidDescriptor)
    }

    pub fn increment_ref_count(&mut self, id: FileDescriptionId) -> Result<(), FileTableError> {
        let description = self
            .descriptions
            .get_mut(id.raw())
            .and_then(Option::as_mut)
            .ok_or(FileTableError::InvalidDescriptor)?;
        description.increment_ref_count();
        Ok(())
    }

    /// Drops one descriptor reference. Returns the underlying file only when
    /// this was the last reference and the VFS should close the description.
    pub fn close(
        &mut self,
        id: FileDescriptionId,
    ) -> Result<Option<DescriptorObject>, FileTableError> {
        let slot = self
            .descriptions
            .get_mut(id.raw())
            .ok_or(FileTableError::InvalidDescriptor)?;
        let description = slot.as_mut().ok_or(FileTableError::InvalidDescriptor)?;
        if description.decrement_ref_count() == 0 {
            Ok(slot.take().map(OpenFileDescription::object))
        } else {
            Ok(None)
        }
    }
}
