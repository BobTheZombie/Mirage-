//! Process control structures for the Mirage kernel.

use crate::kernel::fs::{DescriptorFlags, FileDescriptionId, Path, Permissions, MAX_PATH_BYTES};
use crate::subkernel::{Credentials, SecurityLabel};

pub const MAX_PENDING_SIGNALS: usize = 32;
pub const MAX_SIGNAL_NUMBER: usize = 64;
pub const SIGKILL: u8 = 9;
pub const SIGTERM: u8 = 15;
pub const SIGCHLD: u8 = 17;

/// Maximum argument pointers recorded for one exec request.
pub const MAX_EXEC_ARGS: usize = 64;
/// Maximum environment pointers recorded for one exec request.
pub const MAX_EXEC_ENVS: usize = 64;

/// Fixed-size argv/env metadata copied from userspace before an exec decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecVectorMetadata {
    pub base: u64,
    pub count: usize,
    pub truncated: bool,
}

impl ExecVectorMetadata {
    pub const fn empty() -> Self {
        Self {
            base: 0,
            count: 0,
            truncated: false,
        }
    }

    pub const fn new(base: u64, count: usize, truncated: bool) -> Self {
        Self {
            base,
            count,
            truncated,
        }
    }
}

/// Well-known service-daemon classes whose images must be backed by a manifest
/// signature before L2 may authorize privileged credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecServiceDaemon {
    Display,
    Network,
    Input,
    L2Driver,
}

/// Compact model of a signed executable manifest. Mirage stores the signer and
/// manifest digest here; actual cryptographic validation happens while building
/// image metadata, not while mechanically replacing process state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecSignatureMetadata {
    pub signer: &'static str,
    pub manifest_digest: u64,
}

impl ExecSignatureMetadata {
    pub const fn new(signer: &'static str, manifest_digest: u64) -> Self {
        Self {
            signer,
            manifest_digest,
        }
    }
}

/// Filesystem and trust metadata for the image targeted by an exec request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecImageMetadata {
    pub inode: u64,
    pub size: u64,
    pub mode: u16,
    pub entry_point: u64,
    pub stack_pointer: u64,
    pub service_daemon: Option<ExecServiceDaemon>,
    pub signature: Option<ExecSignatureMetadata>,
}

impl ExecImageMetadata {
    pub const fn new(
        inode: u64,
        size: u64,
        mode: u16,
        entry_point: u64,
        stack_pointer: u64,
        service_daemon: Option<ExecServiceDaemon>,
        signature: Option<ExecSignatureMetadata>,
    ) -> Self {
        Self {
            inode,
            size,
            mode,
            entry_point,
            stack_pointer,
            service_daemon,
            signature,
        }
    }

    pub const fn is_executable(self) -> bool {
        (self.mode & 0o111) != 0
    }

    pub const fn is_signed_service_daemon(self) -> bool {
        self.service_daemon.is_some() && self.signature.is_some()
    }
}

/// Fully resolved and policy-ready exec request. Building this structure is the
/// parsing/loading phase; L2 authorization consumes it before any process image
/// mutation is allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecRequest {
    pub caller: ProcessId,
    pub path: ProcessPath,
    pub argv: ExecVectorMetadata,
    pub envp: ExecVectorMetadata,
    pub requested_credentials: Credentials,
    pub image: ExecImageMetadata,
}

impl ExecRequest {
    pub const fn new(
        caller: ProcessId,
        path: ProcessPath,
        argv: ExecVectorMetadata,
        envp: ExecVectorMetadata,
        requested_credentials: Credentials,
        image: ExecImageMetadata,
    ) -> Self {
        Self {
            caller,
            path,
            argv,
            envp,
            requested_credentials,
            image,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProcessGroupId(u64);

impl ProcessGroupId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    pub const fn raw(&self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(u64);

impl SessionId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    pub const fn raw(&self) -> u64 {
        self.0
    }
}

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
    /// The process has exited but is still waitable by its parent.
    Zombie,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    raw: i32,
}

impl ExitStatus {
    pub const fn exited(code: i32) -> Self {
        Self {
            raw: (code & 0xff) << 8,
        }
    }

    pub const fn signaled(signal: u8) -> Self {
        Self {
            raw: (signal as i32) & 0x7f,
        }
    }

    pub const fn raw(self) -> i32 {
        self.raw
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignalMask {
    bits: u64,
}

impl SignalMask {
    pub const EMPTY: Self = Self { bits: 0 };

    pub const fn from_bits(bits: u64) -> Self {
        Self { bits }
    }

    pub const fn bits(self) -> u64 {
        self.bits
    }

    pub const fn contains(self, signal: u8) -> bool {
        signal > 0
            && signal as usize <= MAX_SIGNAL_NUMBER
            && (self.bits & (1u64 << (signal - 1))) != 0
    }

    pub fn insert(&mut self, signal: u8) {
        if signal > 0 && signal as usize <= MAX_SIGNAL_NUMBER {
            self.bits |= 1u64 << (signal - 1);
        }
    }

    pub fn remove(&mut self, signal: u8) {
        if signal > 0 && signal as usize <= MAX_SIGNAL_NUMBER {
            self.bits &= !(1u64 << (signal - 1));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignalAction {
    pub handler: u64,
    pub mask: SignalMask,
    pub flags: u64,
}

impl SignalAction {
    pub const DEFAULT: Self = Self {
        handler: 0,
        mask: SignalMask::EMPTY,
        flags: 0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalQueueError {
    InvalidSignal,
    Full,
}

#[derive(Clone, Copy, Debug)]
pub struct PendingSignalQueue {
    signals: [Option<u8>; MAX_PENDING_SIGNALS],
}

impl PendingSignalQueue {
    pub const fn new() -> Self {
        Self {
            signals: [None; MAX_PENDING_SIGNALS],
        }
    }

    pub fn push(&mut self, signal: u8) -> Result<(), SignalQueueError> {
        if signal == 0 || signal as usize > MAX_SIGNAL_NUMBER {
            return Err(SignalQueueError::InvalidSignal);
        }
        let mut idx = 0usize;
        while idx < MAX_PENDING_SIGNALS {
            if self.signals[idx].is_none() {
                self.signals[idx] = Some(signal);
                return Ok(());
            }
            idx += 1;
        }
        Err(SignalQueueError::Full)
    }

    pub fn take_unmasked(&mut self, mask: SignalMask) -> Option<u8> {
        let mut idx = 0usize;
        while idx < MAX_PENDING_SIGNALS {
            if let Some(signal) = self.signals[idx] {
                if signal == SIGKILL || !mask.contains(signal) {
                    self.signals[idx] = None;
                    return Some(signal);
                }
            }
            idx += 1;
        }
        None
    }

    pub fn clear(&mut self) {
        let mut idx = 0usize;
        while idx < MAX_PENDING_SIGNALS {
            self.signals[idx] = None;
            idx += 1;
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessControlBlock<const MAX_FD: usize> {
    pub pid: ProcessId,
    pub parent: Option<ProcessId>,
    pub process_group: ProcessGroupId,
    pub session: SessionId,
    pub state: ProcessState,
    pub priority: ProcessPriority,
    pub entry_point: u64,
    pub address_space_root: u64,
    pub cpu_time: u128,
    pub security_label: SecurityLabel,
    pub thread_count: u16,
    pub exit_status: Option<ExitStatus>,
    pub files: ProcessFileTable<MAX_FD>,
    pub signal_actions: [SignalAction; MAX_SIGNAL_NUMBER + 1],
    pub pending_signals: PendingSignalQueue,
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
            process_group: ProcessGroupId::new(pid.raw()),
            session: match parent {
                Some(parent_pid) => SessionId::new(parent_pid.raw()),
                None => SessionId::new(pid.raw()),
            },
            state: ProcessState::Ready,
            priority,
            entry_point,
            address_space_root: 0,
            cpu_time: 0,
            security_label: SecurityLabel::public(),
            thread_count: 0,
            exit_status: None,
            files: ProcessFileTable::new(),
            signal_actions: [SignalAction::DEFAULT; MAX_SIGNAL_NUMBER + 1],
            pending_signals: PendingSignalQueue::new(),
        }
    }

    pub fn update_security_label(&mut self, label: SecurityLabel) {
        self.security_label = label;
    }

    pub fn increment_thread_count(&mut self) {
        self.thread_count = self.thread_count.saturating_add(1);
    }

    pub fn mark_zombie(&mut self, status: ExitStatus) {
        self.state = ProcessState::Zombie;
        self.exit_status = Some(status);
    }

    pub fn set_exec_image(&mut self, entry_point: u64, address_space_root: u64) {
        self.entry_point = entry_point;
        self.address_space_root = address_space_root;
        self.pending_signals.clear();
    }

    pub fn set_process_group(&mut self, pgid: ProcessGroupId) {
        self.process_group = pgid;
    }

    pub fn set_session(&mut self, sid: SessionId) {
        self.session = sid;
    }

    pub fn queue_signal(&mut self, signal: u8) -> Result<(), SignalQueueError> {
        self.pending_signals.push(signal)
    }

    pub fn take_deliverable_signal(&mut self, mask: SignalMask) -> Option<u8> {
        self.pending_signals.take_unmasked(mask)
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
