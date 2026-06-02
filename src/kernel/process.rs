//! Process control structures for the Mirage kernel.

use crate::kernel::fs::{DescriptorFlags, FileDescriptionId, Path, Permissions, MAX_PATH_BYTES};
use crate::subkernel::SecurityLabel;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProcessId(u64);

impl ProcessId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    pub const fn raw(&self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    Blocked,
    Terminated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessPriority {
    Critical,
    High,
    Normal,
    Low,
}

impl ProcessPriority {
    pub const fn time_slice(self) -> u8 {
        match self {
            ProcessPriority::Critical => 8,
            ProcessPriority::High => 6,
            ProcessPriority::Normal => 4,
            ProcessPriority::Low => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessFileTableError {
    Full,
    InvalidDescriptor,
}

/// Owned absolute path snapshot used for per-process `cwd` and `root`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessPath {
    bytes: [u8; MAX_PATH_BYTES],
    len: usize,
}

impl ProcessPath {
    pub const fn root() -> Self {
        let mut bytes = [0u8; MAX_PATH_BYTES];
        bytes[0] = b'/';
        Self { bytes, len: 1 }
    }

    pub const fn as_bytes(self) -> [u8; MAX_PATH_BYTES] {
        self.bytes
    }

    pub const fn len(self) -> usize {
        self.len
    }

    pub fn from_path(path: Path<'_>) -> Self {
        let mut bytes = [0u8; MAX_PATH_BYTES];
        let raw = path.as_str().as_bytes();
        bytes[..raw.len()].copy_from_slice(raw);
        Self {
            bytes,
            len: raw.len(),
        }
    }

    pub fn as_str(&self) -> &str {
        // ProcessPath is only built from validated UTF-8 kernel paths.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len]) }
    }
}

/// Per-process descriptor-table entry. The referenced open-file description is
/// stored in the kernel file table and carries the shared file offset/status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileDescriptor {
    description: FileDescriptionId,
    flags: DescriptorFlags,
}

impl FileDescriptor {
    pub const fn new(description: FileDescriptionId, flags: DescriptorFlags) -> Self {
        Self { description, flags }
    }

    pub const fn description(self) -> FileDescriptionId {
        self.description
    }

    pub const fn flags(self) -> DescriptorFlags {
        self.flags
    }

    pub const fn close_on_exec(self) -> bool {
        self.flags.contains(DescriptorFlags::CLOSE_ON_EXEC)
    }

    pub fn set_flags(&mut self, flags: DescriptorFlags) {
        self.flags = flags;
    }
}

/// Fixed-size POSIX-like descriptor table scoped to one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessFileTable<const MAX: usize> {
    descriptors: [Option<FileDescriptor>; MAX],
    cwd: ProcessPath,
    root: ProcessPath,
    umask: Permissions,
}

impl<const MAX: usize> ProcessFileTable<MAX> {
    pub const fn new() -> Self {
        Self {
            descriptors: [None; MAX],
            cwd: ProcessPath::root(),
            root: ProcessPath::root(),
            umask: Permissions::new(0o022, 0, 0),
        }
    }

    pub const fn cwd(&self) -> ProcessPath {
        self.cwd
    }

    pub const fn root(&self) -> ProcessPath {
        self.root
    }

    pub fn set_cwd(&mut self, cwd: ProcessPath) {
        self.cwd = cwd;
    }

    pub fn set_root(&mut self, root: ProcessPath) {
        self.root = root;
    }

    pub const fn umask(&self) -> Permissions {
        self.umask
    }

    pub fn set_umask(&mut self, umask: Permissions) -> Permissions {
        let previous = self.umask;
        self.umask = Permissions::new(umask.bits() & 0o777, 0, 0);
        previous
    }

    pub fn open(
        &mut self,
        description: FileDescriptionId,
        flags: DescriptorFlags,
    ) -> Result<usize, ProcessFileTableError> {
        self.open_at_or_above(description, flags, 0)
    }

    pub fn open_at_or_above(
        &mut self,
        description: FileDescriptionId,
        flags: DescriptorFlags,
        min_fd: usize,
    ) -> Result<usize, ProcessFileTableError> {
        let mut fd = min_fd;
        while fd < MAX {
            if self.descriptors[fd].is_none() {
                self.descriptors[fd] = Some(FileDescriptor::new(description, flags));
                return Ok(fd);
            }
            fd += 1;
        }
        Err(ProcessFileTableError::Full)
    }

    pub fn duplicate_to(
        &mut self,
        fd: usize,
        description: FileDescriptionId,
        flags: DescriptorFlags,
    ) -> Result<Option<FileDescriptor>, ProcessFileTableError> {
        let slot = self
            .descriptors
            .get_mut(fd)
            .ok_or(ProcessFileTableError::InvalidDescriptor)?;
        let previous = slot.replace(FileDescriptor::new(description, flags));
        Ok(previous)
    }

    pub fn get(&self, fd: usize) -> Result<FileDescriptor, ProcessFileTableError> {
        self.descriptors
            .get(fd)
            .and_then(|entry| *entry)
            .ok_or(ProcessFileTableError::InvalidDescriptor)
    }

    pub fn get_mut(&mut self, fd: usize) -> Result<&mut FileDescriptor, ProcessFileTableError> {
        self.descriptors
            .get_mut(fd)
            .and_then(Option::as_mut)
            .ok_or(ProcessFileTableError::InvalidDescriptor)
    }

    pub fn close(&mut self, fd: usize) -> Result<FileDescriptor, ProcessFileTableError> {
        let slot = self
            .descriptors
            .get_mut(fd)
            .ok_or(ProcessFileTableError::InvalidDescriptor)?;
        slot.take().ok_or(ProcessFileTableError::InvalidDescriptor)
    }

    pub fn close_on_exec(&mut self) -> [Option<FileDescriptionId>; MAX] {
        let mut closed = [None; MAX];
        let mut idx = 0usize;
        while idx < MAX {
            if let Some(descriptor) = self.descriptors[idx] {
                if descriptor.close_on_exec() {
                    self.descriptors[idx] = None;
                    closed[idx] = Some(descriptor.description());
                }
            }
            idx += 1;
        }
        closed
    }

    pub fn clear(&mut self) -> [Option<FileDescriptionId>; MAX] {
        let mut closed = [None; MAX];
        let mut idx = 0usize;
        while idx < MAX {
            if let Some(descriptor) = self.descriptors[idx].take() {
                closed[idx] = Some(descriptor.description());
            }
            idx += 1;
        }
        closed
    }

    pub fn descriptors(&self) -> &[Option<FileDescriptor>; MAX] {
        &self.descriptors
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessControlBlock<const MAX_FD: usize> {
    pub pid: ProcessId,
    pub parent: Option<ProcessId>,
    pub state: ProcessState,
    pub priority: ProcessPriority,
    pub entry_point: u64,
    pub address_space_root: u64,
    pub cpu_time: u128,
    pub security_label: SecurityLabel,
    pub thread_count: u16,
    pub files: ProcessFileTable<MAX_FD>,
}

impl<const MAX_FD: usize> ProcessControlBlock<MAX_FD> {
    pub const fn new(
        pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
        parent: Option<ProcessId>,
    ) -> Self {
        Self {
            pid,
            parent,
            state: ProcessState::Ready,
            priority,
            entry_point,
            address_space_root: 0,
            cpu_time: 0,
            security_label: SecurityLabel::public(),
            thread_count: 0,
            files: ProcessFileTable::new(),
        }
    }

    pub fn update_security_label(&mut self, label: SecurityLabel) {
        self.security_label = label;
    }

    pub fn increment_thread_count(&mut self) {
        self.thread_count = self.thread_count.saturating_add(1);
    }

    pub fn decrement_thread_count(&mut self) {
        if self.thread_count > 0 {
            self.thread_count -= 1;
        }
    }
}

impl core::fmt::Display for ProcessId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PartialEq<u64> for ProcessId {
    fn eq(&self, other: &u64) -> bool {
        self.0 == *other
    }
}
