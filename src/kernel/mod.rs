//! Core kernel primitives: process lifecycle, scheduling, IPC routing, and
//! multi-core orchestration.

pub mod block;
pub mod boot_diagnostics;
pub mod boot_phase;
pub mod boot_runtime;
pub mod boot_screen;
pub mod boot_status;
pub mod cpu;
pub mod debug_shell;
pub mod device;
pub mod dispatch;
pub mod elf;
pub mod exec;
pub mod fs;
pub mod futex;
pub mod input;
pub mod ipc;
pub mod memory;
pub mod mmio;
pub mod partition;
pub mod platform;
pub mod process;
pub mod root;
pub mod services;
pub mod spider_pid1;
pub mod sync;
pub mod syscall;
pub mod thread;
pub mod time;
pub mod timer;
pub mod userspace;

use crate::arch::x86_64::{
    self,
    boot::{BootInfo, BootModules, FramebufferInfo},
    clock, ThreadRunOutcome, ThreadSliceRunContext,
};
use crate::kernel::boot_phase::{
    boot_phase_detected, boot_phase_failed, boot_phase_online, boot_phase_skipped,
    boot_phase_start, BootPhase,
};
use crate::kernel::cpu::CpuCoreState;
use crate::kernel::device::{
    DeviceDescriptor, DeviceError as DriverError, DeviceId, DeviceKind, DeviceManager,
    MirageDeviceDescriptor,
};
use crate::kernel::exec::{CloneTaskRequest, SpawnTaskRequest};
use crate::kernel::fs::inode::InodeKind;
use crate::kernel::fs::{
    open_flags_from_libc, permissions_from_libc_mode, syscall_error_code_from_vfs, AccessMode,
    CDirEntry, CStat, DescriptorFlags, DescriptorObject, DirEntry, EventFdId, Ext4Backend,
    FileDescriptionId, FileSystem, FileTable, FileTableError, FsCredentials, Path, PathError,
    PipeDirection, PipeEndpoint, PipeId, QfsFileSystem, SocketHandle, SsdUsbOptions, SuperBlock,
    VfsError, MAX_PATH_BYTES,
};
use crate::kernel::futex::{FutexKey, FutexTable, MAX_FUTEX_WAITERS};
use crate::kernel::ipc::{Message, MessagePayload, MessageQueue, MessageQueueError};
use crate::kernel::memory::MemoryProtection;
use crate::kernel::process::{
    ExecRequest, ExecServiceDaemon, ExecSignatureMetadata, ExecVectorMetadata, ExitStatus,
    ProcessControlBlock, ProcessFileTableError, ProcessGroupId, ProcessId, ProcessPath,
    ProcessPriority, ProcessState, SessionId, SignalAction, SignalMask, MAX_EXEC_ARGS,
    MAX_EXEC_ENVS, MAX_SUPPLEMENTARY_GROUPS, SIGCHLD, SIGKILL, SIGTERM,
};
use crate::kernel::services::network::{
    NetworkIpcRequest, NetworkOpcode, NetworkRecvmsgRequest, NetworkRequestHeader,
    NetworkSendmsgRequest, NetworkSockaddrRequest, NetworkSocketRequest,
};
use crate::kernel::services::registry::{
    ServiceId as RegistryServiceId, ServiceRegistry, ServiceRegistryError, MAX_DEVICE_CLAIMS,
    MAX_SERVICE_REGISTRATIONS,
};
use crate::kernel::syscall::{
    SyscallContext, SyscallErrorCode, SyscallNumber, MIRAGE_SYSCALL_ERROR_BIT,
};
use crate::kernel::thread::{
    CpuContext, PrivilegeMode, ThreadControlBlock, ThreadId, ThreadState, MAX_THREADS,
};
use crate::kernel::time::KERNEL_TIME;
use crate::kernel::timer::{TimerError, TimerManager, MAX_PROCESS_TIMERS, MAX_SLEEP_ENTRIES};
use crate::subkernel::{
    CapabilityId, CapabilityObject, CapabilityRight, CapabilityRights, Credentials, DeviceSecurity,
    IsolationError, SecurityClass, SecurityKernel,
};
use core::cmp::min;
use core::ptr::NonNull;
use mirage_mtss::{
    AddressSpaceId as MtssAddressSpaceId, CoreMtss, CoreMtssError, CoreTask, CoreTaskId,
    CoreThread, CpuId as MtssCpuId, Mtss, MtssConfig, MtssError, MtssThreadScheduleRecord,
    Priority as MtssPriority, ScheduleDecision, StackRange, TaskId as MtssTaskId,
    ThreadId as MtssThreadId, Timeslice as MtssTimeslice, UserProgramImage, UserThreadPreflight,
};

pub type KernelThreadScheduleRecord =
    MtssThreadScheduleRecord<ThreadId, ProcessId, ProcessPriority>;

pub const MAX_PROCESSES: usize = 64;
pub const MESSAGE_DEPTH: usize = 16;
pub const MAX_DEVICES: usize = 12;
pub const MAX_OPEN_FILES: usize = 64;
pub const MAX_KERNEL_PIPES: usize = 32;
pub const MAX_KERNEL_EVENTFDS: usize = 32;
const PIPE_BUFFER_BYTES: usize = 4096;

const AT_FDCWD: i32 = -100;
const SEEK_SET: u64 = 0;
const SEEK_CUR: u64 = 1;
const SEEK_END: u64 = 2;
const AT_REMOVEDIR: u64 = 0x200;
const AT_SYMLINK_FOLLOW: u64 = 0x400;
const RENAME_NOREPLACE: u64 = 1;
const F_DUPFD: u64 = 0;
const F_GETFD: u64 = 1;
const F_SETFD: u64 = 2;
const F_GETFL: u64 = 3;
const F_DUPFD_CLOEXEC: u64 = 1030;
const FD_CLOEXEC: u64 = 1;
const O_CLOEXEC_RAW: u64 = 0o02000000;
const O_NONBLOCK_RAW: u64 = 0o0004000;
const O_DIRECT_RAW: u64 = 0o0040000;
const PIPE2_SUPPORTED_FLAGS: u64 = O_CLOEXEC_RAW | O_NONBLOCK_RAW;
const EFD_SEMAPHORE: u64 = 1;
const EFD_CLOEXEC: u64 = O_CLOEXEC_RAW;
const EFD_NONBLOCK: u64 = O_NONBLOCK_RAW;
const EVENTFD_SUPPORTED_FLAGS: u64 = EFD_SEMAPHORE | EFD_CLOEXEC | EFD_NONBLOCK;
const POLLIN: i16 = 0x0001;
const POLLPRI: i16 = 0x0002;
const POLLOUT: i16 = 0x0004;
const POLLERR: i16 = 0x0008;
const POLLHUP: i16 = 0x0010;
const POLLNVAL: i16 = 0x0020;
const FIONREAD: u64 = 0x541b;
const BLKSSZGET: u64 = 0x1268;
const BLKGETSIZE64: u64 = 0x80081272;
const MIRAGE_IOCTL_DEVICE_INFO: u64 = 0x4d01;
const FUTEX_WAIT: u64 = 0;
const FUTEX_WAKE: u64 = 1;
const FUTEX_PRIVATE_FLAG: u64 = 0x80;
const FUTEX_CMD_MASK: u64 = !(FUTEX_PRIVATE_FLAG);
const ARCH_SET_GS: u64 = 0x1001;
const ARCH_SET_FS: u64 = 0x1002;
const ARCH_GET_FS: u64 = 0x1003;
const ARCH_GET_GS: u64 = 0x1004;
const USER_CANONICAL_LIMIT: u64 = 0x0000_8000_0000_0000;

const DEFAULT_ROOT_FILESYSTEM: &[u8] = b"qfs";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PipeObject {
    buffer: [u8; PIPE_BUFFER_BYTES],
    head: usize,
    len: usize,
    readers: u16,
    writers: u16,
}

impl PipeObject {
    const fn new() -> Self {
        Self {
            buffer: [0; PIPE_BUFFER_BYTES],
            head: 0,
            len: 0,
            readers: 1,
            writers: 1,
        }
    }

    const fn is_readable(self) -> bool {
        self.len > 0 || self.writers == 0
    }

    const fn is_writable(self) -> bool {
        self.readers > 0 && self.len < PIPE_BUFFER_BYTES
    }

    fn read(&mut self, out: &mut [u8]) -> usize {
        let count = min(out.len(), self.len);
        let mut idx = 0usize;
        while idx < count {
            out[idx] = self.buffer[self.head];
            self.head = (self.head + 1) % PIPE_BUFFER_BYTES;
            idx += 1;
        }
        self.len -= count;
        count
    }

    fn write(&mut self, data: &[u8]) -> usize {
        let count = min(data.len(), PIPE_BUFFER_BYTES - self.len);
        let mut idx = 0usize;
        while idx < count {
            let tail = (self.head + self.len) % PIPE_BUFFER_BYTES;
            self.buffer[tail] = data[idx];
            self.len += 1;
            idx += 1;
        }
        count
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EventFdObject {
    counter: u64,
    semaphore: bool,
}

impl EventFdObject {
    const fn new(counter: u64, semaphore: bool) -> Self {
        Self { counter, semaphore }
    }

    const fn is_readable(self) -> bool {
        self.counter > 0
    }

    const fn is_writable(self) -> bool {
        self.counter <= u64::MAX - 2
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MiragePollFd {
    pub fd: i32,
    pub events: i16,
    pub revents: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootMountSource {
    BuiltInBlockQfs,
    BootModuleQfs { index: u64 },
    BootModuleExt4 { index: u64 },
    DiscoveredBlockQfs,
    DiscoveredBlockExt4,
}

enum RootFileSystem {
    Qfs(QfsFileSystem),
    Ext4(Ext4Backend<'static>),
}

impl RootFileSystem {
    pub const fn new() -> Self {
        Self::Qfs(QfsFileSystem::new_on_block_device(
            false,
            crate::kernel::device::built_in_block_storage(),
        ))
    }

    fn mount_qfs(
        &mut self,
        read_only: bool,
        device: &'static dyn crate::kernel::device::BlockStorageDevice,
    ) -> Result<(), VfsError> {
        let next = QfsFileSystem::new_on_block_device(read_only, device);
        next.refresh_from_block_device()?;
        *self = Self::Qfs(next);
        Ok(())
    }

    fn mount_ext4(
        &mut self,
        device: &'static dyn crate::kernel::device::BlockStorageDevice,
    ) -> Result<(), VfsError> {
        boot_phase_start(BootPhase::Ext4);
        let mut superblock = [0u8; 1024];
        if let Err(error) = read_ext4_superblock(device, &mut superblock) {
            boot_phase_skipped(BootPhase::Ext4, "superblock unavailable");
            return Err(error);
        }
        let backend =
            match Ext4Backend::mount(device, &superblock, SsdUsbOptions::flash_friendly(8)) {
                Ok(backend) => backend,
                Err(crate::kernel::fs::Ext4Error::BadMagic) => {
                    boot_phase_skipped(BootPhase::Ext4, "no ext4 superblock");
                    return Err(VfsError::InvalidSuperblock);
                }
                Err(_) => {
                    boot_phase_failed(BootPhase::Ext4, "mount failed");
                    return Err(VfsError::Unsupported);
                }
            };
        boot_phase_detected(BootPhase::Ext4);
        *self = Self::Ext4(backend);
        boot_phase_online(BootPhase::Ext4);
        Ok(())
    }
}

impl FileSystem for RootFileSystem {
    fn root_inode(&self) -> crate::kernel::fs::InodeId {
        match self {
            Self::Qfs(fs) => fs.root_inode(),
            Self::Ext4(fs) => fs.root_inode(),
        }
    }
    fn super_block(&self) -> SuperBlock {
        match self {
            Self::Qfs(fs) => fs.super_block(),
            Self::Ext4(fs) => fs.super_block(),
        }
    }
    fn lookup(&self, path: Path<'_>) -> Result<crate::kernel::fs::InodeMetadata, VfsError> {
        match self {
            Self::Qfs(fs) => fs.lookup(path),
            Self::Ext4(fs) => fs.lookup(path),
        }
    }
    fn open(
        &self,
        path: Path<'_>,
        flags: crate::kernel::fs::OpenFlags,
        credentials: FsCredentials,
    ) -> Result<crate::kernel::fs::File, VfsError> {
        match self {
            Self::Qfs(fs) => fs.open(path, flags, credentials),
            Self::Ext4(fs) => fs.open(path, flags, credentials),
        }
    }
    fn close(&self, file: crate::kernel::fs::File) -> Result<(), VfsError> {
        match self {
            Self::Qfs(fs) => fs.close(file),
            Self::Ext4(fs) => fs.close(file),
        }
    }
    fn pread(
        &self,
        file: &crate::kernel::fs::File,
        buffer: &mut [u8],
        offset: u64,
    ) -> Result<usize, VfsError> {
        match self {
            Self::Qfs(fs) => fs.pread(file, buffer, offset),
            Self::Ext4(fs) => fs.pread(file, buffer, offset),
        }
    }
    fn pwrite(
        &self,
        file: &crate::kernel::fs::File,
        data: &[u8],
        offset: u64,
    ) -> Result<usize, VfsError> {
        match self {
            Self::Qfs(fs) => fs.pwrite(file, data, offset),
            Self::Ext4(fs) => fs.pwrite(file, data, offset),
        }
    }
    fn mkdir(
        &self,
        path: Path<'_>,
        mode: crate::kernel::fs::Permissions,
        credentials: FsCredentials,
    ) -> Result<(), VfsError> {
        match self {
            Self::Qfs(fs) => fs.mkdir(path, mode, credentials),
            Self::Ext4(fs) => fs.mkdir(path, mode, credentials),
        }
    }
    fn rmdir(&self, path: Path<'_>, credentials: FsCredentials) -> Result<(), VfsError> {
        match self {
            Self::Qfs(fs) => fs.rmdir(path, credentials),
            Self::Ext4(fs) => fs.rmdir(path, credentials),
        }
    }
    fn unlink(&self, path: Path<'_>, credentials: FsCredentials) -> Result<(), VfsError> {
        match self {
            Self::Qfs(fs) => fs.unlink(path, credentials),
            Self::Ext4(fs) => fs.unlink(path, credentials),
        }
    }
    fn rename(
        &self,
        old_path: Path<'_>,
        new_path: Path<'_>,
        credentials: FsCredentials,
    ) -> Result<(), VfsError> {
        match self {
            Self::Qfs(fs) => fs.rename(old_path, new_path, credentials),
            Self::Ext4(fs) => fs.rename(old_path, new_path, credentials),
        }
    }
    fn link(
        &self,
        old_path: Path<'_>,
        new_path: Path<'_>,
        credentials: FsCredentials,
    ) -> Result<(), VfsError> {
        match self {
            Self::Qfs(fs) => fs.link(old_path, new_path, credentials),
            Self::Ext4(fs) => fs.link(old_path, new_path, credentials),
        }
    }
    fn symlink(
        &self,
        target: &str,
        link_path: Path<'_>,
        credentials: FsCredentials,
    ) -> Result<(), VfsError> {
        match self {
            Self::Qfs(fs) => fs.symlink(target, link_path, credentials),
            Self::Ext4(fs) => fs.symlink(target, link_path, credentials),
        }
    }
    fn readlink(&self, path: Path<'_>, buffer: &mut [u8]) -> Result<usize, VfsError> {
        match self {
            Self::Qfs(fs) => fs.readlink(path, buffer),
            Self::Ext4(fs) => fs.readlink(path, buffer),
        }
    }
    fn stat(&self, path: Path<'_>) -> Result<crate::kernel::fs::Stat, VfsError> {
        match self {
            Self::Qfs(fs) => fs.stat(path),
            Self::Ext4(fs) => fs.stat(path),
        }
    }
    fn fstat(&self, file: &crate::kernel::fs::File) -> Result<crate::kernel::fs::Stat, VfsError> {
        match self {
            Self::Qfs(fs) => fs.fstat(file),
            Self::Ext4(fs) => fs.fstat(file),
        }
    }
    fn lookup_inode(
        &self,
        inode: crate::kernel::fs::InodeId,
    ) -> Result<crate::kernel::fs::InodeMetadata, VfsError> {
        match self {
            Self::Qfs(fs) => fs.lookup_inode(inode),
            Self::Ext4(fs) => fs.lookup_inode(inode),
        }
    }
    fn chmod(&self, path: Path<'_>, mode: u16, credentials: FsCredentials) -> Result<(), VfsError> {
        match self {
            Self::Qfs(fs) => fs.chmod(path, mode, credentials),
            Self::Ext4(fs) => fs.chmod(path, mode, credentials),
        }
    }
    fn chown(
        &self,
        path: Path<'_>,
        uid: u16,
        gid: u16,
        credentials: FsCredentials,
    ) -> Result<(), VfsError> {
        match self {
            Self::Qfs(fs) => fs.chown(path, uid, gid, credentials),
            Self::Ext4(fs) => fs.chown(path, uid, gid, credentials),
        }
    }
    fn truncate(
        &self,
        path: Path<'_>,
        size: u64,
        credentials: FsCredentials,
    ) -> Result<(), VfsError> {
        match self {
            Self::Qfs(fs) => fs.truncate(path, size, credentials),
            Self::Ext4(fs) => fs.truncate(path, size, credentials),
        }
    }
    fn ftruncate(
        &self,
        file: &crate::kernel::fs::File,
        size: u64,
        credentials: FsCredentials,
    ) -> Result<(), VfsError> {
        match self {
            Self::Qfs(fs) => fs.ftruncate(file, size, credentials),
            Self::Ext4(fs) => fs.ftruncate(file, size, credentials),
        }
    }
    fn fsync(&self, file: &crate::kernel::fs::File) -> Result<(), VfsError> {
        match self {
            Self::Qfs(fs) => fs.fsync(file),
            Self::Ext4(fs) => fs.fsync(file),
        }
    }
    fn readdir(
        &self,
        path: Path<'_>,
        offset: usize,
        entries: &mut [DirEntry],
    ) -> Result<usize, VfsError> {
        match self {
            Self::Qfs(fs) => fs.readdir(path, offset, entries),
            Self::Ext4(fs) => fs.readdir(path, offset, entries),
        }
    }
    fn readdir_inode(
        &self,
        inode: crate::kernel::fs::InodeId,
        offset: usize,
        entries: &mut [DirEntry],
    ) -> Result<usize, VfsError> {
        match self {
            Self::Qfs(fs) => fs.readdir_inode(inode, offset, entries),
            Self::Ext4(fs) => fs.readdir_inode(inode, offset, entries),
        }
    }
}

#[derive(Clone, Copy)]
struct BootModuleBlockState {
    base: *const u8,
    size: usize,
}

unsafe impl Send for BootModuleBlockState {}

struct BootModuleBlockDevice {
    state: crate::kernel::sync::SpinLock<BootModuleBlockState>,
}

impl BootModuleBlockDevice {
    const fn new() -> Self {
        Self {
            state: crate::kernel::sync::SpinLock::new(BootModuleBlockState {
                base: core::ptr::null(),
                size: 0,
            }),
        }
    }

    fn configure(&self, base: *const u8, size: usize) {
        *self.state.lock() = BootModuleBlockState { base, size };
    }
}

impl crate::kernel::device::BlockStorageDevice for BootModuleBlockDevice {
    fn sector_size(&self) -> usize {
        512
    }
    fn sector_count(&self) -> u64 {
        (self.state.lock().size / 512) as u64
    }
    fn read_sectors(
        &self,
        first_sector: u64,
        buffer: &mut [u8],
    ) -> Result<usize, crate::kernel::device::DeviceError> {
        if buffer.len() % 512 != 0 {
            return Err(crate::kernel::device::DeviceError::BufferTooSmall);
        }
        let state = self.state.lock();
        let start = (first_sector as usize)
            .checked_mul(512)
            .ok_or(crate::kernel::device::DeviceError::Unsupported)?;
        let end = start
            .checked_add(buffer.len())
            .ok_or(crate::kernel::device::DeviceError::Unsupported)?;
        if state.base.is_null() || end > state.size {
            return Err(crate::kernel::device::DeviceError::BufferTooSmall);
        }
        let src = unsafe { core::slice::from_raw_parts(state.base.add(start), buffer.len()) };
        buffer.copy_from_slice(src);
        Ok(buffer.len())
    }
    fn write_sectors(
        &self,
        _first_sector: u64,
        _data: &[u8],
    ) -> Result<usize, crate::kernel::device::DeviceError> {
        Err(crate::kernel::device::DeviceError::Unsupported)
    }
    fn flush(&self) -> Result<(), crate::kernel::device::DeviceError> {
        Ok(())
    }
    fn discard(
        &self,
        _first_sector: u64,
        _sector_count: u64,
    ) -> Result<(), crate::kernel::device::DeviceError> {
        Err(crate::kernel::device::DeviceError::Unsupported)
    }
}

static BOOT_MODULE_BLOCK_DEVICE: BootModuleBlockDevice = BootModuleBlockDevice::new();

fn read_ext4_superblock(
    device: &'static dyn crate::kernel::device::BlockStorageDevice,
    out: &mut [u8; 1024],
) -> Result<(), VfsError> {
    let sector_size = device.sector_size();
    if sector_size == 0 || sector_size > out.len() || 1024 % sector_size != 0 {
        return Err(VfsError::Unsupported);
    }
    let first_sector = (1024 / sector_size) as u64;
    let sectors = out.len() / sector_size;
    device
        .read_sectors(first_sector, &mut out[..sectors * sector_size])
        .map_err(|_| VfsError::Unsupported)?;
    Ok(())
}

struct KernelPathBuf {
    bytes: [u8; MAX_PATH_BYTES],
    len: usize,
}

impl KernelPathBuf {
    fn from_str(raw: &str) -> KernelResult<Self> {
        let mut bytes = [0u8; MAX_PATH_BYTES];
        if raw.is_empty() || raw.len() > MAX_PATH_BYTES {
            return Err(KernelError::Filesystem(VfsError::InvalidPath(
                PathError::TooLong,
            )));
        }
        bytes[..raw.len()].copy_from_slice(raw.as_bytes());
        Ok(Self {
            bytes,
            len: raw.len(),
        })
    }

    fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len]) }
    }

    fn as_path(&self) -> KernelResult<Path<'_>> {
        Path::new(self.as_str()).map_err(map_path_error)
    }

    fn truncate_to_root(&mut self, root_len: usize) {
        self.len = root_len.max(1);
    }

    fn push_component(&mut self, component: &str) -> KernelResult<()> {
        if component.is_empty() || component == "." {
            return Ok(());
        }
        if component == ".." {
            while self.len > 1 && self.bytes[self.len - 1] == b'/' {
                self.len -= 1;
            }
            while self.len > 1 && self.bytes[self.len - 1] != b'/' {
                self.len -= 1;
            }
            if self.len > 1 {
                self.len -= 1;
            }
            return Ok(());
        }
        if component.len() > crate::kernel::fs::MAX_COMPONENT_BYTES {
            return Err(KernelError::Filesystem(VfsError::NameTooLong));
        }
        let extra = if self.len == 1 {
            component.len()
        } else {
            component.len() + 1
        };
        if self.len + extra > MAX_PATH_BYTES {
            return Err(KernelError::Filesystem(VfsError::InvalidPath(
                PathError::TooLong,
            )));
        }
        if self.len != 1 {
            self.bytes[self.len] = b'/';
            self.len += 1;
        }
        self.bytes[self.len..self.len + component.len()].copy_from_slice(component.as_bytes());
        self.len += component.len();
        Ok(())
    }
}

fn is_supported_root_filesystem(filesystem_type: &[u8]) -> bool {
    matches!(filesystem_type, b"qfs" | b"ext4" | b"ssd_usb" | b"ssd-usb")
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirageTimespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirageItimerspec {
    pub it_interval: MirageTimespec,
    pub it_value: MirageTimespec,
}

#[derive(Debug, Clone, Copy)]
pub enum KernelError {
    ProcessTableFull,
    SchedulerFull,
    UnknownProcess,
    UnknownThread,
    ThreadTableFull,
    MessageQueueFull,
    MessageQueueEmpty,
    SecurityViolation(IsolationError),
    IsolationFault(IsolationError),
    DeviceNotFound,
    DeviceFault(DriverError),
    InvalidSyscall,
    InvalidArgument,
    InvalidPointer,
    AllocationFailed,
    FileTableFull,
    Filesystem(VfsError),
    TimedOut,
    Loader(crate::kernel::userspace::LoadError),
}

pub type KernelResult<T> = core::result::Result<T, KernelError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MtssInitReport {
    pub core_ready: bool,
    pub scheduler_ready: bool,
    pub timer_ready: bool,
    pub preemption_ready: bool,
    pub idle_ready: bool,
    pub api_ready: bool,
}

impl MtssInitReport {
    pub const fn required_components_ready(&self) -> bool {
        self.core_ready
            && self.scheduler_ready
            && self.timer_ready
            && self.preemption_ready
            && self.idle_ready
            && self.api_ready
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessExitReport {
    pub pid: ProcessId,
    pub status: ExitStatus,
}

const EMPTY_DEVICE_DESCRIPTOR: DeviceDescriptor = DeviceDescriptor::new(
    DeviceId::new(0),
    DeviceKind::SerialConsole,
    "",
    DeviceSecurity::new(SecurityClass::Public, false),
);

pub struct Kernel<const MAX_PROC: usize, const MSG_DEPTH: usize> {
    process_table: [Option<ProcessControlBlock<MAX_OPEN_FILES>>; MAX_PROC],
    ipc_queues: [MessageQueue<MSG_DEPTH>; MAX_PROC],
    mtss_scheduler: Mtss<MAX_PROCESSES, MAX_THREADS, MAX_THREADS, MAX_THREADS>,
    mtss_core: CoreMtss<MAX_PROCESSES, MAX_THREADS, MAX_THREADS>,
    mtss_initialized: bool,
    mtss_ticks: u64,
    pending_mtss_decision: Option<KernelThreadScheduleRecord>,
    security: SecurityKernel<MAX_PROC>,
    devices: DeviceManager<MAX_DEVICES>,
    service_registry: ServiceRegistry<MAX_SERVICE_REGISTRATIONS, MAX_DEVICE_CLAIMS>,
    root_fs: RootFileSystem,
    open_files: FileTable<MAX_OPEN_FILES>,
    core_states: [CpuCoreState; cpu::MAX_CORES],
    thread_table: [Option<ThreadControlBlock>; MAX_THREADS],
    timers: TimerManager<MAX_SLEEP_ENTRIES, MAX_PROCESS_TIMERS>,
    pipes: [Option<PipeObject>; MAX_KERNEL_PIPES],
    eventfds: [Option<EventFdObject>; MAX_KERNEL_EVENTFDS],
    futexes: FutexTable<MAX_FUTEX_WAITERS>,
    next_pid: u64,
    next_thread: u64,
    message_sequence: u64,
    next_socket_handle: u64,
}

impl<const MAX_PROC: usize, const MSG_DEPTH: usize> Kernel<MAX_PROC, MSG_DEPTH> {
    const THREAD_CAPACITY: usize = MAX_THREADS;

    const fn schedule_record(
        thread: ThreadId,
        process: ProcessId,
        priority: ProcessPriority,
    ) -> KernelThreadScheduleRecord {
        MtssThreadScheduleRecord::new(thread, process, priority, priority.time_slice())
    }

    const fn new_mtss_scheduler() -> Mtss<MAX_PROCESSES, MAX_THREADS, MAX_THREADS, MAX_THREADS> {
        Mtss::new(
            MtssConfig::new(MtssCpuId::new(0)).with_default_timeslice(MtssTimeslice::from_ticks(4)),
        )
    }

    const fn mtss_task_id(pid: ProcessId) -> MtssTaskId {
        MtssTaskId::new(pid.raw())
    }

    const fn mtss_thread_id(thread: ThreadId) -> MtssThreadId {
        MtssThreadId::new(thread.raw())
    }

    const fn mtss_priority(priority: ProcessPriority) -> MtssPriority {
        match priority {
            ProcessPriority::Critical => MtssPriority::CRITICAL,
            ProcessPriority::High => MtssPriority::HIGH,
            ProcessPriority::Normal => MtssPriority::NORMAL,
            ProcessPriority::Low => MtssPriority::LOW,
        }
    }

    fn schedule_record_from_mtss(
        &self,
        decision: ScheduleDecision,
    ) -> Option<KernelThreadScheduleRecord> {
        let thread = ThreadId::new(decision.next.raw());
        let index = self.locate_thread(thread).ok()?;
        let tcb = self.thread_table[index]?;
        Some(Self::schedule_record(tcb.id, tcb.process, tcb.priority))
    }

    pub(super) fn mtss_create_task(
        &mut self,
        pid: ProcessId,
        parent: Option<ProcessId>,
        address_space_root: u64,
        priority: ProcessPriority,
    ) -> KernelResult<()> {
        self.mtss_scheduler
            .create_task(
                Self::mtss_task_id(pid),
                parent.map(Self::mtss_task_id),
                MtssAddressSpaceId::new(if address_space_root == 0 {
                    pid.raw()
                } else {
                    address_space_root
                }),
                Self::mtss_priority(priority),
            )
            .map(|_| ())
            .map_err(map_mtss_error)
    }

    pub(super) fn mtss_create_thread(
        &mut self,
        pid: ProcessId,
        thread: ThreadId,
        priority: ProcessPriority,
    ) -> KernelResult<()> {
        self.mtss_scheduler
            .create_thread(
                Self::mtss_task_id(pid),
                Self::mtss_thread_id(thread),
                Self::mtss_priority(priority),
            )
            .map(|_| ())
            .map_err(map_mtss_error)
    }

    pub(super) fn mtss_enqueue_thread(&mut self, thread: ThreadId) -> KernelResult<()> {
        self.mtss_scheduler
            .enqueue_thread(Self::mtss_thread_id(thread))
            .map_err(map_mtss_error)
    }

    pub const fn new() -> Self {
        Self {
            process_table: [None; MAX_PROC],
            ipc_queues: [MessageQueue::new(); MAX_PROC],
            mtss_scheduler: Self::new_mtss_scheduler(),
            mtss_core: CoreMtss::new(),
            mtss_initialized: false,
            mtss_ticks: 0,
            pending_mtss_decision: None,
            security: SecurityKernel::new(),
            devices: DeviceManager::new(),
            service_registry: ServiceRegistry::new(),
            root_fs: RootFileSystem::new(),
            open_files: FileTable::new(),
            core_states: [CpuCoreState::new(); cpu::MAX_CORES],
            thread_table: [None; MAX_THREADS],
            timers: TimerManager::new(),
            pipes: [None; MAX_KERNEL_PIPES],
            eventfds: [None; MAX_KERNEL_EVENTFDS],
            futexes: FutexTable::new(),
            next_pid: 1,
            next_thread: 1,
            message_sequence: 0,
            next_socket_handle: 1,
        }
    }

    pub fn bootstrap(&mut self) {
        self.bootstrap_with_framebuffer(None);
    }

    pub fn bootstrap_with_framebuffer(&mut self, framebuffer: Option<FramebufferInfo>) {
        let _ = self.bootstrap_with_boot_info_and_framebuffer(None, framebuffer);
    }

    pub fn bootstrap_with_boot_info(&mut self, boot_info: &BootInfo) -> KernelResult<()> {
        self.bootstrap_with_boot_info_and_framebuffer(Some(boot_info), boot_info.framebuffer)
    }

    fn bootstrap_with_boot_info_and_framebuffer(
        &mut self,
        boot_info: Option<&BootInfo>,
        framebuffer: Option<FramebufferInfo>,
    ) -> KernelResult<()> {
        self.mtss_scheduler = Self::new_mtss_scheduler();
        self.mtss_core = CoreMtss::new();
        self.mtss_initialized = false;
        self.mtss_ticks = 0;
        self.pending_mtss_decision = None;
        self.security.reset();
        self.devices.reset();
        self.service_registry.reset();
        self.open_files.clear();
        self.timers.reset();
        self.pipes = [None; MAX_KERNEL_PIPES];
        self.eventfds = [None; MAX_KERNEL_EVENTFDS];
        self.futexes.reset();
        self.next_pid = 1;
        self.next_thread = 1;
        self.message_sequence = 0;
        self.next_socket_handle = 1;
        KERNEL_TIME.init(clock::DEFAULT_FREQUENCY_HZ);

        let mut idx = 0;
        while idx < MAX_PROC {
            self.process_table[idx] = None;
            self.ipc_queues[idx].clear();
            idx += 1;
        }

        idx = 0;
        while idx < Self::THREAD_CAPACITY {
            self.thread_table[idx] = None;
            idx += 1;
        }

        idx = 0;
        while idx < cpu::MAX_CORES {
            self.core_states[idx] = CpuCoreState::new();
            idx += 1;
        }
        idx = 0;
        while idx < cpu::MAX_CORES {
            self.core_states[idx].set_kernel_stack_top(x86_64::kernel_stack_top(idx));
            idx += 1;
        }
        if cpu::MAX_CORES > 0 {
            self.core_states[0].online();
        }

        if let Some(boot_info) = boot_info {
            self.devices
                .install_core_devices_with_boot_info(Some(boot_info))
                .map_err(KernelError::DeviceFault)?;
        } else {
            self.devices
                .install_core_devices_with_framebuffer(framebuffer)
                .map_err(KernelError::DeviceFault)?;
        }
        Ok(())
    }

    pub fn mount_root_from_boot_sources(
        &mut self,
        modules: BootModules,
    ) -> KernelResult<RootMountSource> {
        let mut index = 0u64;
        while index < modules.len() {
            if let Some(module) = modules.module(index) {
                if module.size >= 512 {
                    BOOT_MODULE_BLOCK_DEVICE
                        .configure(module.base.0 as *const u8, module.size as usize);
                    if self
                        .root_fs
                        .mount_qfs(true, &BOOT_MODULE_BLOCK_DEVICE)
                        .is_ok()
                    {
                        return Ok(RootMountSource::BootModuleQfs { index });
                    }
                    if self.root_fs.mount_ext4(&BOOT_MODULE_BLOCK_DEVICE).is_ok() {
                        return Ok(RootMountSource::BootModuleExt4 { index });
                    }
                }
            }
            index += 1;
        }

        let mut descriptors = [EMPTY_DEVICE_DESCRIPTOR; MAX_DEVICES];
        let count = self.devices.enumerate(&mut descriptors);
        let mut device_index = 0usize;
        while device_index < count {
            let descriptor = descriptors[device_index];
            if descriptor.kind == DeviceKind::BlockStorage {
                if let Ok(device) = self.devices.block_storage_static(descriptor.id) {
                    if self.root_fs.mount_ext4(device).is_ok() {
                        return Ok(RootMountSource::DiscoveredBlockExt4);
                    }
                    if self.root_fs.mount_qfs(false, device).is_ok() {
                        return Ok(RootMountSource::DiscoveredBlockQfs);
                    }
                }
            }
            device_index += 1;
        }

        let built_in = crate::kernel::device::built_in_block_storage();
        if self.root_fs.mount_ext4(built_in).is_ok() {
            return Ok(RootMountSource::DiscoveredBlockExt4);
        }
        if self.root_fs.mount_qfs(false, built_in).is_ok() {
            return Ok(RootMountSource::DiscoveredBlockQfs);
        }
        Ok(RootMountSource::BuiltInBlockQfs)
    }

    pub fn bootstrap_userspace_init(&mut self) -> KernelResult<(ProcessId, &'static str)> {
        const INIT_CANDIDATES: [&str; 4] =
            ["/sbin/spider-rs", "/sbin/init", "/bin/init", "/bin/sh"];
        let init = self.spawn_initial_process(Credentials::system())?;
        let mut idx = 0usize;
        while idx < INIT_CANDIDATES.len() {
            match self.exec_bootstrap_path(init, INIT_CANDIDATES[idx]) {
                Ok(()) => return Ok((init, INIT_CANDIDATES[idx])),
                Err(_) => {
                    idx += 1;
                }
            }
        }
        self.terminate_process(init);
        Err(KernelError::Filesystem(VfsError::NotFound))
    }

    fn exec_bootstrap_path(&mut self, pid: ProcessId, raw_path: &str) -> KernelResult<()> {
        let resolved = KernelPathBuf::from_str(raw_path)?;
        let path = resolved.as_path()?;
        let stat = self.root_fs.stat(path).map_err(KernelError::Filesystem)?;
        if stat.kind != InodeKind::RegularFile {
            return Err(KernelError::Filesystem(VfsError::PermissionDenied));
        }
        let argv = ExecVectorMetadata::empty();
        let envp = ExecVectorMetadata::empty();
        let image = self.load_exec_image(pid, &resolved, stat, argv, envp)?;
        let request = ExecRequest::new(
            pid,
            ProcessPath::from_path(path),
            argv,
            envp,
            Credentials::system(),
            image,
        );
        self.exec_task(request, None)
    }

    pub fn bring_up_secondary_cores(&mut self, count: usize) {
        let mut brought_online = 0usize;
        let mut idx = 1usize;
        while idx < cpu::MAX_CORES && brought_online < count {
            self.core_states[idx].online();
            brought_online += 1;
            idx += 1;
        }
    }

    pub fn online_core_count(&self) -> usize {
        let mut count = 0usize;
        let mut idx = 0usize;
        while idx < cpu::MAX_CORES {
            if self.core_states[idx].online {
                count += 1;
            }
            idx += 1;
        }
        count
    }

    /// Initialize the kernel-facing MTSS integration without installing a
    /// CPU-specific timer/preemption backend for this milestone.
    pub fn kernel_mtss_init(&mut self) -> Result<MtssInitReport, KernelError> {
        self.mtss_core = CoreMtss::new();
        self.mtss_scheduler = Self::new_mtss_scheduler();
        self.mtss_initialized = false;
        self.mtss_ticks = 0;
        self.pending_mtss_decision = None;

        let kernel_stack_top = x86_64::kernel_stack_top(0);
        let kernel_stack =
            StackRange::new(kernel_stack_top.saturating_sub(0x4000), kernel_stack_top);
        self.mtss_core
            .init_with_idle(kernel_stack)
            .map_err(map_core_mtss_error)?;

        self.mtss_initialized = true;

        Ok(MtssInitReport {
            core_ready: true,
            scheduler_ready: true,
            timer_ready: false,
            preemption_ready: false,
            idle_ready: true,
            api_ready: true,
        })
    }

    /// Let MTSS account the timer tick before kernel-owned timer, wakeup, and
    /// interrupt-delivery mechanics run.
    ///
    /// The MTSS call is the interrupt-path accounting API: it advances MTSS time,
    /// decrements the running micro-thread's time slice, sets `need_resched` on
    /// expiry, and avoids blocking or heap allocation.  This milestone has no
    /// architecture preemption-disable counter wired into `Kernel`, so the tick
    /// is reported as preemptible; MTSS still supports deferred rescheduling when
    /// a backend passes `preemption_disabled = true`.
    pub fn kernel_on_timer_tick(&mut self) {
        if self.mtss_initialized {
            self.mtss_ticks = self.mtss_ticks.saturating_add(1);
            let _ = self
                .mtss_scheduler
                .on_timer_tick_with_preemption_disabled(false);
        }
    }

    /// Ask MTSS for the next runnable micro-thread. CPU entry, address-space
    /// switching, syscall entry, and capability checks remain kernel-owned.
    pub fn kernel_schedule_next(&mut self) -> Option<KernelThreadScheduleRecord> {
        if let Some(decision) = self.pending_mtss_decision.take() {
            return Some(decision);
        }
        self.mtss_scheduler
            .pick_next()
            .ok()
            .flatten()
            .and_then(|decision| self.schedule_record_from_mtss(decision))
    }

    /// Attempt the Spider-rs PID 1 launch path without faking ring-3 entry.
    ///
    /// The current milestone validates the ordering and loader availability. If
    /// the ELF/rootfs byte path or architecture userspace entry is missing, the
    /// caller must mark Spider-rs as `Stub` or `Failed`, not `Online`.
    pub fn bootstrap_spider_rs_pid1(&mut self) -> KernelResult<ProcessId> {
        if !self.mtss_initialized {
            return Err(KernelError::InvalidArgument);
        }
        match crate::kernel::userspace::load_elf_from_file("/sbin/spider-rs") {
            Ok(_program) => Err(KernelError::InvalidArgument),
            Err(_) => Err(KernelError::Filesystem(VfsError::NotFound)),
        }
    }

    /// Launch Spider-rs from the immutable Boot Runtime image as a userspace task.
    ///
    /// This path reads bytes from RAMFS, validates the ELF entry, and creates an
    /// MTSS-visible process. It never calls Spider-rs as a kernel function.
    pub fn bootstrap_spider_rs_pid1_from_image(&mut self, image: &[u8]) -> KernelResult<ProcessId> {
        if !self.mtss_initialized {
            return Err(KernelError::InvalidArgument);
        }
        let parsed = crate::kernel::userspace::elf_loader::parse_elf64(image)
            .map_err(KernelError::Loader)?;
        crate::kernel::userspace::elf_loader::validate_elf64(image).map_err(KernelError::Loader)?;
        self.admit_pid1_through_mtss(image, parsed)
    }

    fn admit_pid1_through_mtss(
        &mut self,
        image: &[u8],
        parsed: crate::kernel::userspace::elf_loader::ParsedElf,
    ) -> KernelResult<ProcessId> {
        #[cfg(test)]
        let _ = image;
        let entry_point = parsed.entry.0;
        self.dump_pid1_elf_diagnostics(parsed, false, 0, false);
        #[cfg(test)]
        let address_space_root =
            mirage_mtss::AddressSpaceId::new(CoreTaskId::FIRST_USERSPACE.raw()).raw();
        #[cfg(not(test))]
        let address_space_root =
            crate::kernel::memory::create_user_address_space(ProcessId::new(1))
                .ok_or(KernelError::AllocationFailed)?;
        let address_space = mirage_mtss::AddressSpaceId::new(address_space_root);
        #[cfg(not(test))]
        self.map_pid1_elf_image(ProcessId::new(1), address_space_root, image, parsed)?;
        #[cfg(not(test))]
        let stack = crate::kernel::userspace::memory::allocate_user_stack(address_space, 0x20_000)
            .map_err(|_| {
                KernelError::Loader(crate::kernel::userspace::LoadError::StackBuildFailed)
            })?;
        #[cfg(not(test))]
        let stack_top = self
            .build_pid1_initial_stack(address_space_root, stack)
            .map_err(KernelError::Loader)?;
        #[cfg(test)]
        let stack_top = 0x0000_7fff_feff_ffc8;
        #[cfg(test)]
        let stack = crate::kernel::userspace::memory::UserStack {
            bottom: crate::kernel::userspace::memory::VirtAddr(0x0000_7fff_fee0_0000),
            top: crate::kernel::userspace::memory::VirtAddr(0x0000_7fff_ff00_0000),
            size: 0x20_000,
        };
        #[cfg(not(test))]
        let mapping_preflight =
            self.preflight_pid1_user_entry(address_space_root, entry_point, stack_top)?;
        #[cfg(test)]
        let mapping_preflight = UserThreadPreflight {
            canonical_rip: entry_point < USER_CANONICAL_LIMIT,
            canonical_rsp: stack_top < USER_CANONICAL_LIMIT && (stack_top & 0xf) == 0,
            executable_user_rip_mapping: true,
            writable_user_stack_mapping: true,
            valid_user_cs: true,
            valid_user_ss: true,
            valid_kernel_stack: true,
            valid_tss_rsp0: true,
            valid_address_space: address_space_root != 0,
            valid_cr3: address_space_root != 0,
        };
        #[cfg(not(test))]
        self.dump_pid1_elf_diagnostics(parsed, true, stack_top, true);
        let user_stack = StackRange::new(stack.bottom.0, stack.top.0);
        let kernel_stack_top = x86_64::kernel_stack_top(0).saturating_sub(0x4000);
        if kernel_stack_top == 0 {
            return Err(KernelError::InvalidArgument);
        }
        let user_entry_preflight = UserThreadPreflight {
            valid_user_cs: crate::arch::x86_64::gdt::USER_CODE_SELECTOR == 0x1b,
            valid_user_ss: crate::arch::x86_64::gdt::USER_DATA_SELECTOR == 0x23,
            valid_kernel_stack: kernel_stack_top != 0,
            valid_tss_rsp0: crate::arch::x86_64::gdt::tss_rsp0() != 0,
            ..mapping_preflight
        };
        let mtss_task = self
            .mtss_core
            .spawn_userspace(
                "spider-rs",
                UserProgramImage {
                    entry: entry_point,
                    address_space,
                    user_stack,
                    cr3: address_space_root,
                },
                StackRange::new(kernel_stack_top.saturating_sub(0x4000), kernel_stack_top),
                user_entry_preflight,
            )
            .map_err(map_core_mtss_error)?;

        let pid = self.spawn_task(SpawnTaskRequest {
            parent: None,
            entry_point,
            priority: ProcessPriority::Normal,
            credentials: Credentials::system(),
        })?;
        if pid.raw() != mtss_task.raw() {
            return Err(KernelError::InvalidArgument);
        }
        let index = self.locate_process(pid)?;
        if let Some(pcb) = self.process_table[index].as_mut() {
            pcb.address_space_root = address_space_root;
        }
        self.configure_pid1_user_frame(pid, entry_point, stack_top)?;
        Ok(pid)
    }

    fn configure_pid1_user_frame(
        &mut self,
        pid: ProcessId,
        entry_point: u64,
        stack_pointer: u64,
    ) -> KernelResult<()> {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(tcb) = self.thread_table[idx].as_mut() {
                if tcb.process == pid {
                    tcb.entry_point = entry_point;
                    tcb.stack_pointer = stack_pointer;
                    tcb.context = CpuContext::new(entry_point, stack_pointer, PrivilegeMode::User);
                    tcb.context.rdi = 0;
                    tcb.context.rsi = 0;
                    tcb.context.rdx = 0;
                    return Ok(());
                }
            }
            idx += 1;
        }
        Err(KernelError::UnknownThread)
    }

    fn preflight_pid1_user_entry(
        &self,
        address_space_root: u64,
        entry: u64,
        stack_pointer: u64,
    ) -> KernelResult<UserThreadPreflight> {
        if address_space_root == 0
            || entry == 0
            || entry >= USER_CANONICAL_LIMIT
            || stack_pointer == 0
            || stack_pointer >= USER_CANONICAL_LIMIT
            || (stack_pointer & 0xf) != 0
        {
            return Err(KernelError::InvalidArgument);
        }
        let entry_prot =
            crate::kernel::memory::find_user_mapping_protection(address_space_root, entry, 1)
                .ok_or(KernelError::Loader(
                    crate::kernel::userspace::LoadError::EntryNotMapped,
                ))?;
        if !entry_prot.read || !entry_prot.execute {
            return Err(KernelError::Loader(
                crate::kernel::userspace::LoadError::EntryNotMapped,
            ));
        }
        let stack_probe = stack_pointer.saturating_sub(8);
        let stack_prot =
            crate::kernel::memory::find_user_mapping_protection(address_space_root, stack_probe, 8)
                .ok_or(KernelError::Loader(
                    crate::kernel::userspace::LoadError::StackBuildFailed,
                ))?;
        if !stack_prot.read || !stack_prot.write {
            return Err(KernelError::Loader(
                crate::kernel::userspace::LoadError::StackBuildFailed,
            ));
        }
        crate::arch::x86_64::early_console::panic_write_fmt(format_args!(
            "[user-entry preflight]\npid=1\nrip={:#x}\nrsp={:#x}\ncr3={:#x}\ncs=ring3-validated-by-arch-pending\nss=ring3-validated-by-arch-pending\nrflags=0x202\nentry_mapped=true\nstack_mapped=true\n",
            entry, stack_pointer, address_space_root
        ));
        Ok(UserThreadPreflight {
            canonical_rip: true,
            canonical_rsp: true,
            executable_user_rip_mapping: true,
            writable_user_stack_mapping: true,
            valid_user_cs: false,
            valid_user_ss: false,
            valid_kernel_stack: false,
            valid_tss_rsp0: false,
            valid_address_space: true,
            valid_cr3: true,
        })
    }

    fn dump_pid1_elf_diagnostics(
        &self,
        parsed: crate::kernel::userspace::elf_loader::ParsedElf,
        entry_mapped: bool,
        stack_pointer: u64,
        stack_mapped: bool,
    ) {
        crate::arch::x86_64::early_console::panic_write_fmt(format_args!(
            "[pid1-loader] ELF entry: {:#x} entry mapped: {} stack: {:#x} stack mapped: {}\n",
            parsed.entry.0, entry_mapped, stack_pointer, stack_mapped
        ));
        let mut idx = 0usize;
        while idx < parsed.segment_count {
            let segment = parsed.segments[idx];
            crate::arch::x86_64::early_console::panic_write_fmt(format_args!(
                "[pid1-loader] PT_LOAD[{}]: vaddr={:#x} memsz={:#x} filesz={:#x} flags={:#x}\n",
                idx, segment.vaddr.0, segment.mem_size, segment.file_size, segment.flags
            ));
            idx += 1;
        }
    }

    fn map_pid1_elf_image(
        &mut self,
        owner: ProcessId,
        address_space_root: u64,
        image: &[u8],
        parsed: crate::kernel::userspace::elf_loader::ParsedElf,
    ) -> KernelResult<()> {
        let mut idx = 0usize;
        while idx < parsed.segment_count {
            let segment = parsed.segments[idx];
            let mapping = crate::kernel::userspace::elf_loader::segment_mapping(
                segment.vaddr,
                segment.file_offset,
                segment.mem_size,
            );
            let base = mapping.map_start;
            let len = mapping.map_len;
            let protection =
                MemoryProtection::new(true, segment.flags & 0x2 != 0, segment.flags & 0x1 != 0);
            let mapped = crate::kernel::memory::mmap_user_fixed(
                owner,
                address_space_root,
                base.0,
                len,
                protection,
            )
            .ok_or(KernelError::Loader(
                crate::kernel::userspace::LoadError::MapSegmentFailed,
            ))?;

            let page_offset = segment.vaddr.0.saturating_sub(base.0) as usize;
            unsafe {
                core::ptr::write_bytes(mapped.as_ptr(), 0, mapped.length);
                core::ptr::copy_nonoverlapping(
                    image.as_ptr().add(segment.file_offset),
                    mapped.as_ptr().add(page_offset),
                    segment.file_size,
                );
            }
            idx += 1;
        }
        Ok(())
    }

    fn build_pid1_initial_stack(
        &self,
        address_space_root: u64,
        stack: crate::kernel::userspace::memory::UserStack,
    ) -> Result<u64, crate::kernel::userspace::LoadError> {
        let region = crate::kernel::memory::find_user_mapping(
            address_space_root,
            stack.bottom.0,
            stack.size,
            true,
        )
        .ok_or(crate::kernel::userspace::LoadError::StackBuildFailed)?;
        let mut sp = stack.top.0 & !0xf;
        let argv = b"spider-rs\0";
        sp = sp
            .checked_sub(argv.len() as u64)
            .ok_or(crate::kernel::userspace::LoadError::StackBuildFailed)?;
        let argv0 = sp;
        let argv_offset = (argv0 - stack.bottom.0) as usize;
        unsafe {
            core::ptr::copy_nonoverlapping(
                argv.as_ptr(),
                region.as_ptr().add(argv_offset),
                argv.len(),
            );
        }
        sp &= !0xf;
        push_user_u64(region.as_ptr(), stack.bottom.0, &mut sp, 0)?; // AT_NULL
        push_user_u64(region.as_ptr(), stack.bottom.0, &mut sp, 0)?; // auxv value
        push_user_u64(region.as_ptr(), stack.bottom.0, &mut sp, 0)?; // envp terminator
        push_user_u64(region.as_ptr(), stack.bottom.0, &mut sp, 0)?; // argv terminator
        push_user_u64(region.as_ptr(), stack.bottom.0, &mut sp, argv0)?;
        push_user_u64(region.as_ptr(), stack.bottom.0, &mut sp, 1)?;
        Ok(sp)
    }

    pub fn mtss_pid1_task(&self) -> Option<CoreTask> {
        self.mtss_core.task(CoreTaskId::FIRST_USERSPACE)
    }

    pub fn mtss_pid1_main_thread(&self) -> Option<CoreThread> {
        let task = self.mtss_pid1_task()?;
        self.mtss_core.thread(task.main_thread)
    }

    /// Complete a kernel-dispatched slice and hand scheduling authority back to MTSS.
    ///
    /// The kernel records the completed slice on its schedule record, but MTSS owns
    /// runnable-state transitions and selects the next runnable thread. Architecture
    /// backends must receive only a thread selected by MTSS, never a kernel-local
    /// requeue choice.
    pub fn kernel_yield_current(
        &mut self,
        mut scheduled: KernelThreadScheduleRecord,
    ) -> Result<Option<KernelThreadScheduleRecord>, KernelError> {
        let _ = scheduled.consume_time_slice();
        let decision = self
            .mtss_scheduler
            .yield_current()
            .map_err(map_mtss_error)?;
        match decision {
            Some(decision) => self
                .schedule_record_from_mtss(decision)
                .map(Some)
                .ok_or(KernelError::UnknownThread),
            None => Ok(None),
        }
    }

    pub fn spawn_initial_process(&mut self, creds: Credentials) -> KernelResult<ProcessId> {
        self.spawn_task(SpawnTaskRequest {
            parent: None,
            entry_point: 0,
            priority: ProcessPriority::Critical,
            credentials: creds,
        })
    }

    pub fn spawn_child_process(
        &mut self,
        parent_pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
        requested_creds: Credentials,
    ) -> KernelResult<ProcessId> {
        self.spawn_task(SpawnTaskRequest {
            parent: Some(parent_pid),
            entry_point,
            priority,
            credentials: requested_creds,
        })
    }

    pub fn spawn_thread(
        &mut self,
        pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
    ) -> KernelResult<ThreadId> {
        self.clone_thread(CloneTaskRequest::legacy_thread(
            pid,
            None,
            entry_point,
            priority,
        ))
    }

    pub fn terminate_process(&mut self, pid: ProcessId) {
        self.exit_process(pid, ExitStatus::signaled(SIGTERM));
    }

    pub fn exit_process(
        &mut self,
        pid: ProcessId,
        status: ExitStatus,
    ) -> Option<ProcessExitReport> {
        if let Ok(index) = self.locate_process(pid) {
            if let Some(pcb) = self.process_table[index].as_ref() {
                if pcb.state == ProcessState::Zombie {
                    return None;
                }
            }
            if let Some(mut pcb) = self.process_table[index].take() {
                self.release_process_file_table(&mut pcb.files);
                pcb.mark_zombie(status);
                self.process_table[index] = Some(pcb);
            }
            self.ipc_queues[index].clear();
            let _ = self.mtss_scheduler.terminate_task(Self::mtss_task_id(pid));
            self.remove_threads_for_process(pid);
            memory::release_process(pid);
            self.security.revoke_task(pid);
            self.timers.release_process(pid);
            self.futexes.remove_owner(self.futex_owner_for_process(pid));
            let _ = self.queue_signal_to_parent(pid, SIGCHLD);
            return Some(ProcessExitReport { pid, status });
        }
        None
    }

    pub fn terminate_thread(&mut self, thread: ThreadId) {
        if let Ok(index) = self.locate_thread(thread) {
            if let Some(tcb) = self.thread_table[index] {
                let _ = self
                    .mtss_scheduler
                    .exit_thread(Self::mtss_thread_id(thread));
                self.futexes.remove_thread(thread);
                self.remove_thread_from_cores(thread);
                self.thread_table[index] = None;
                self.update_process_thread_count(tcb.process, false);
            }
        }
    }

    pub fn register_service(
        &mut self,
        authorizer: ProcessId,
        service: RegistryServiceId,
        owner: ProcessId,
    ) -> KernelResult<()> {
        self.security
            .authorize_service_control(authorizer)
            .map_err(KernelError::SecurityViolation)?;
        self.security
            .authorize_service_registration(owner, service.security_class())
            .map_err(KernelError::SecurityViolation)?;
        self.ensure_process_exists(owner)?;
        self.service_registry
            .register(service, owner)
            .map_err(map_service_registry_error)
    }

    pub fn register_endpoint(
        &mut self,
        authorizer: ProcessId,
        service: RegistryServiceId,
        owner: ProcessId,
    ) -> KernelResult<()> {
        self.register_service(authorizer, service, owner)
    }

    pub fn revoke_task(&mut self, pid: ProcessId) {
        self.security.revoke_task(pid);
    }

    pub fn grant_task_capability(
        &mut self,
        owner: ProcessId,
        object: CapabilityObject,
        rights: CapabilityRights,
    ) -> KernelResult<CapabilityId> {
        self.security
            .grant_capability(owner, object, rights)
            .map_err(KernelError::SecurityViolation)
    }

    pub fn revoke_task_capability(&mut self, capability: CapabilityId) -> KernelResult<()> {
        self.security
            .revoke_capability(capability)
            .map_err(KernelError::SecurityViolation)
    }

    pub fn revoke_task_capabilities(&mut self, owner: ProcessId) {
        self.security.revoke_all_capabilities(owner);
    }

    pub fn revoke_service_owner(&mut self, owner: ProcessId) {
        self.service_registry.revoke_owner(owner);
    }

    pub fn check_service_control_capability(&self, pid: ProcessId) -> KernelResult<()> {
        self.security
            .authorize_service_control(pid)
            .map_err(KernelError::SecurityViolation)
    }

    pub fn check_service_registration_capability(
        &self,
        owner: ProcessId,
        service: RegistryServiceId,
    ) -> KernelResult<()> {
        self.security
            .authorize_service_registration(owner, service.security_class())
            .map_err(KernelError::SecurityViolation)
    }

    pub fn service_owner(&self, service: RegistryServiceId) -> Option<ProcessId> {
        self.service_registry.owner(service)
    }

    pub fn send_service_message(
        &mut self,
        sender: ProcessId,
        service: RegistryServiceId,
        payload: MessagePayload,
    ) -> KernelResult<()> {
        let receiver = self
            .service_registry
            .owner(service)
            .ok_or(KernelError::UnknownProcess)?;
        self.send_message(sender, receiver, payload)
    }

    pub fn claim_service_device(
        &mut self,
        owner: ProcessId,
        service: RegistryServiceId,
        device: DeviceId,
    ) -> KernelResult<()> {
        let descriptor = self
            .devices
            .descriptor(device)
            .ok_or(KernelError::DeviceNotFound)?;
        self.security
            .authorize_device_access(
                owner,
                CapabilityObject::PciDevice(descriptor.id.raw() as u64),
                CapabilityRight::Control,
                descriptor.security,
            )
            .map_err(KernelError::SecurityViolation)?;
        self.service_registry
            .claim_device(service, owner, descriptor)
            .map_err(map_service_registry_error)
    }

    pub fn release_service_device(
        &mut self,
        owner: ProcessId,
        service: RegistryServiceId,
        device: DeviceId,
    ) -> KernelResult<()> {
        self.service_registry
            .release_device(service, owner, device)
            .map_err(map_service_registry_error)
    }

    pub fn send_message(
        &mut self,
        sender: ProcessId,
        receiver: ProcessId,
        payload: MessagePayload,
    ) -> KernelResult<()> {
        self.security
            .authorize_ipc(sender, receiver, payload.security_class)
            .map_err(KernelError::SecurityViolation)?;

        let message = Message::new(sender, receiver, self.next_message_sequence(), payload);
        let queue_index = self.locate_process(receiver)?;
        self.ipc_queues[queue_index]
            .push(message)
            .map_err(|MessageQueueError::Full| KernelError::MessageQueueFull)?;

        let mut wake_threads = false;
        if let Some(pcb) = self.process_table[queue_index].as_mut() {
            if pcb.state == ProcessState::Blocked {
                pcb.state = ProcessState::Ready;
                wake_threads = true;
            }
        }

        if wake_threads {
            if let Err(err) = self.make_threads_ready(receiver) {
                // Sending to a blocked process is transactional: if the wakeup cannot be
                // scheduled, the receiver stays blocked and the just-enqueued message is
                // removed so callers can retry without duplicating delivery.
                if let Some(pcb) = self.process_table[queue_index].as_mut() {
                    pcb.state = ProcessState::Blocked;
                }
                let _ = self.ipc_queues[queue_index].rollback_last_push();
                return Err(err);
            }
        }

        Ok(())
    }

    pub fn receive_message(&mut self, pid: ProcessId) -> KernelResult<Message> {
        let queue_index = self.locate_process(pid)?;
        self.ipc_queues[queue_index]
            .pop()
            .ok_or(KernelError::MessageQueueEmpty)
    }

    pub fn receive_or_block(&mut self, pid: ProcessId) -> KernelResult<Option<Message>> {
        let queue_index = self.locate_process(pid)?;
        if let Some(message) = self.ipc_queues[queue_index].pop() {
            return Ok(Some(message));
        }

        self.block_process_at_index(pid, queue_index);
        Ok(None)
    }

    pub fn block_for_message(&mut self, pid: ProcessId) {
        if let Ok(index) = self.locate_process(pid) {
            self.block_process_at_index(pid, index);
        }
    }

    pub fn wait(&mut self, parent: ProcessId, status: Option<&mut i32>) -> KernelResult<ProcessId> {
        let status_ptr = status
            .map(|out| out as *mut i32)
            .unwrap_or(core::ptr::null_mut());
        self.wait_for_child(parent, -1, status_ptr, 0)
            .map(ProcessId::new)
    }

    pub fn waitpid(
        &mut self,
        parent: ProcessId,
        pid: i64,
        status: Option<&mut i32>,
        options: u64,
    ) -> KernelResult<ProcessId> {
        let status_ptr = status
            .map(|out| out as *mut i32)
            .unwrap_or(core::ptr::null_mut());
        self.wait_for_child(parent, pid, status_ptr, options)
            .map(ProcessId::new)
    }

    pub fn queue_thread_syscall(
        &mut self,
        thread: ThreadId,
        number: u64,
        args: [u64; syscall::SYSCALL_MAX_ARGS],
    ) -> KernelResult<()> {
        let index = self.locate_thread(thread)?;
        if let Some(tcb) = self.thread_table[index].as_mut() {
            tcb.prepare_syscall(number, args);
            Ok(())
        } else {
            Err(KernelError::UnknownThread)
        }
    }

    pub fn thread_context(&self, thread: ThreadId) -> KernelResult<CpuContext> {
        let index = self.locate_thread(thread)?;
        self.thread_table[index]
            .map(|tcb| tcb.context)
            .ok_or(KernelError::UnknownThread)
    }

    pub fn handle_syscall(&mut self, number: u64, context: SyscallContext) -> KernelResult<u64> {
        match SyscallNumber::from_raw(number).ok_or(KernelError::InvalidSyscall)? {
            SyscallNumber::GetPid => Ok(context.caller.raw()),
            SyscallNumber::Spawn => self.syscall_spawn(context),
            SyscallNumber::SendIpc => self.syscall_send_ipc(context),
            SyscallNumber::ReceiveIpc => self.syscall_receive_ipc(context),
            SyscallNumber::ReceiveOrBlockIpc => self.syscall_receive_or_block_ipc(context),
            SyscallNumber::BlockForIpc => {
                self.security
                    .authorize_ipc_receive(context.caller)
                    .map_err(KernelError::SecurityViolation)?;
                self.block_for_message(context.caller);
                Ok(0)
            }
            SyscallNumber::EnumerateDevices => self.syscall_enumerate_devices(context),
            SyscallNumber::DeviceInfo => self.syscall_device_info(context),
            SyscallNumber::DeviceRead => self.syscall_device_read(context),
            SyscallNumber::DeviceWrite => self.syscall_device_write(context),
            SyscallNumber::Mmap => self.syscall_mmap(context),
            SyscallNumber::Munmap => self.syscall_munmap(context),
            SyscallNumber::Malloc => self.syscall_malloc(context),
            SyscallNumber::Free => self.syscall_free(context),
            SyscallNumber::Realloc => self.syscall_realloc(context),
            SyscallNumber::MallocAligned => self.syscall_malloc_aligned(context),
            SyscallNumber::OpenAt => self.syscall_openat(context),
            SyscallNumber::Close => self.syscall_close(context),
            SyscallNumber::Read => self.syscall_read(context),
            SyscallNumber::Write => self.syscall_write(context),
            SyscallNumber::Pread64 => self.syscall_pread64(context),
            SyscallNumber::Pwrite64 => self.syscall_pwrite64(context),
            SyscallNumber::Lseek => self.syscall_lseek(context),
            SyscallNumber::Statx => self.syscall_statx(context),
            SyscallNumber::NewFstatAt => self.syscall_newfstatat(context),
            SyscallNumber::Getdents64 => self.syscall_getdents64(context),
            SyscallNumber::MkdirAt => self.syscall_mkdirat(context),
            SyscallNumber::UnlinkAt => self.syscall_unlinkat(context),
            SyscallNumber::RenameAt2 => self.syscall_renameat2(context),
            SyscallNumber::Ftruncate => self.syscall_ftruncate(context),
            SyscallNumber::Fsync => self.syscall_fsync(context),
            SyscallNumber::Mount => self.syscall_mount(context),
            SyscallNumber::Chdir => self.syscall_chdir(context),
            SyscallNumber::Fchdir => self.syscall_fchdir(context),
            SyscallNumber::Getcwd => self.syscall_getcwd(context),
            SyscallNumber::Faccessat => self.syscall_faccessat(context),
            SyscallNumber::Fchmodat => self.syscall_fchmodat(context),
            SyscallNumber::Fchownat => self.syscall_fchownat(context),
            SyscallNumber::Symlinkat => self.syscall_symlinkat(context),
            SyscallNumber::Readlinkat => self.syscall_readlinkat(context),
            SyscallNumber::Linkat => self.syscall_linkat(context),
            SyscallNumber::RegisterService => self.syscall_register_service(context),
            SyscallNumber::SendServiceIpc => self.syscall_send_service_ipc(context),
            SyscallNumber::ClaimDevice => self.syscall_claim_device(context),
            SyscallNumber::ReleaseDevice => self.syscall_release_device(context),
            SyscallNumber::Fork => self.syscall_fork(context),
            SyscallNumber::Execve => self.syscall_execve(context),
            SyscallNumber::Exit => self.syscall_exit(context),
            SyscallNumber::Wait4 => self.syscall_wait4(context),
            SyscallNumber::GetPpid => self.syscall_getppid(context),
            SyscallNumber::SetPgid => self.syscall_setpgid(context),
            SyscallNumber::Setsid => self.syscall_setsid(context),
            SyscallNumber::GetUid => self.syscall_getuid(context),
            SyscallNumber::GetEuid => self.syscall_geteuid(context),
            SyscallNumber::SetUid => self.syscall_setuid(context),
            SyscallNumber::GetGid => self.syscall_getgid(context),
            SyscallNumber::SetGid => self.syscall_setgid(context),
            SyscallNumber::GetGroups => self.syscall_getgroups(context),
            SyscallNumber::SetGroups => self.syscall_setgroups(context),
            SyscallNumber::RtSigaction => self.syscall_rt_sigaction(context),
            SyscallNumber::RtSigprocmask => self.syscall_rt_sigprocmask(context),
            SyscallNumber::Kill => self.syscall_kill(context),
            SyscallNumber::RtSigreturn => self.syscall_rt_sigreturn(context),
            SyscallNumber::ClockGettime => self.syscall_clock_gettime(context),
            SyscallNumber::Nanosleep => self.syscall_nanosleep(context),
            SyscallNumber::TimerCreate => self.syscall_timer_create(context),
            SyscallNumber::TimerSettime => self.syscall_timer_settime(context),
            SyscallNumber::TimerGettime => self.syscall_timer_gettime(context),
            SyscallNumber::TimerDelete => self.syscall_timer_delete(context),
            SyscallNumber::Dup => self.syscall_dup(context),
            SyscallNumber::Dup2 => self.syscall_dup2(context),
            SyscallNumber::Dup3 => self.syscall_dup3(context),
            SyscallNumber::Fcntl => self.syscall_fcntl(context),
            SyscallNumber::Ioctl => self.syscall_ioctl(context),
            SyscallNumber::Pipe2 => self.syscall_pipe2(context),
            SyscallNumber::Poll => self.syscall_poll(context),
            SyscallNumber::Pselect => self.syscall_pselect(context),
            SyscallNumber::Eventfd => self.syscall_eventfd(context),
            SyscallNumber::Socket => self.syscall_socket(context),
            SyscallNumber::Bind => self.syscall_bind(context),
            SyscallNumber::Listen => self.syscall_listen(context),
            SyscallNumber::Accept => self.syscall_accept(context),
            SyscallNumber::Connect => self.syscall_connect(context),
            SyscallNumber::Sendmsg => self.syscall_sendmsg(context),
            SyscallNumber::Recvmsg => self.syscall_recvmsg(context),
            SyscallNumber::Clone => self.syscall_clone(context),
            SyscallNumber::Futex => self.syscall_futex(context),
            SyscallNumber::SetThreadArea => self.syscall_set_thread_area(context),
            SyscallNumber::ArchPrctl => self.syscall_arch_prctl(context),
            SyscallNumber::Yield => self.syscall_yield(context),
        }
    }

    fn syscall_yield(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.ensure_process_exists(context.caller)?;
        let thread = context.thread.ok_or(KernelError::UnknownThread)?;
        let thread_index = self.locate_thread(thread)?;
        let tcb = self.thread_table[thread_index]
            .as_mut()
            .ok_or(KernelError::UnknownThread)?;
        if tcb.process != context.caller {
            return Err(KernelError::SecurityViolation(
                IsolationError::PolicyViolation,
            ));
        }
        if tcb.state == ThreadState::Terminated {
            return Err(KernelError::UnknownThread);
        }
        if tcb.state == ThreadState::Running {
            tcb.mark_ready();
        }
        Ok(0)
    }

    fn syscall_fork(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.fork_task(context.caller, context.thread)
            .map(|pid| pid.raw())
    }

    fn syscall_execve(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let resolved = self.resolve_user_path(context.caller, AT_FDCWD as u64, context.arg(0))?;
        let path = resolved.as_path()?;
        let stat = self.root_fs.stat(path).map_err(KernelError::Filesystem)?;
        if stat.kind != InodeKind::RegularFile {
            return Err(KernelError::Filesystem(VfsError::PermissionDenied));
        }
        if !crate::kernel::fs::Permissions::new(stat.mode, stat.uid, stat.gid).allows(
            self.fs_credentials_for(context.caller)?,
            AccessMode::Execute,
        ) {
            return Err(KernelError::Filesystem(VfsError::PermissionDenied));
        }

        let argv = exec_vector_metadata(context.arg(1), MAX_EXEC_ARGS)?;
        let envp = exec_vector_metadata(context.arg(2), MAX_EXEC_ENVS)?;
        if argv.truncated || envp.truncated {
            return Err(KernelError::InvalidArgument);
        }
        let image = self.load_exec_image(context.caller, &resolved, stat, argv, envp)?;
        let request = ExecRequest::new(
            context.caller,
            ProcessPath::from_path(path),
            argv,
            envp,
            decode_credentials(context.arg(5))?,
            image,
        );

        self.exec_task(request, context.thread)?;
        Ok(0)
    }

    fn syscall_exit(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.exit_task(context.caller, ExitStatus::exited(context.arg(0) as i32));
        Ok(0)
    }

    fn syscall_wait4(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.wait_task(
            context.caller,
            context.arg(0) as i64,
            context.arg(1) as *mut i32,
            context.arg(2),
        )
    }

    fn syscall_getppid(&self, context: SyscallContext) -> KernelResult<u64> {
        let index = self.locate_process(context.caller)?;
        Ok(self.process_table[index]
            .as_ref()
            .and_then(|pcb| pcb.parent)
            .map(|pid| pid.raw())
            .unwrap_or(0))
    }

    fn syscall_setpgid(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let target = if context.arg(0) == 0 {
            context.caller
        } else {
            ProcessId::new(context.arg(0))
        };
        let pgid = if context.arg(1) == 0 {
            ProcessGroupId::new(target.raw())
        } else {
            ProcessGroupId::new(context.arg(1))
        };
        let caller_index = self.locate_process(context.caller)?;
        let caller_session = self.process_table[caller_index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .session;
        let target_index = self.locate_process(target)?;
        let target_pcb = self.process_table[target_index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?;
        if target != context.caller && target_pcb.parent != Some(context.caller) {
            return Err(KernelError::InvalidArgument);
        }
        if target_pcb.session != caller_session {
            return Err(KernelError::InvalidArgument);
        }
        target_pcb.set_process_group(pgid);
        Ok(0)
    }

    fn syscall_setsid(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let index = self.locate_process(context.caller)?;
        let sid = SessionId::new(context.caller.raw());
        let pgid = ProcessGroupId::new(context.caller.raw());
        let pcb = self.process_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?;
        if pcb.process_group.raw() == context.caller.raw() {
            return Err(KernelError::InvalidArgument);
        }
        pcb.set_session(sid);
        pcb.set_process_group(pgid);
        Ok(sid.raw())
    }

    fn syscall_getuid(&self, context: SyscallContext) -> KernelResult<u64> {
        Ok(self.process_credentials(context.caller)?.uid as u64)
    }

    fn syscall_geteuid(&self, context: SyscallContext) -> KernelResult<u64> {
        Ok(self.process_credentials(context.caller)?.euid as u64)
    }

    fn syscall_getgid(&self, context: SyscallContext) -> KernelResult<u64> {
        Ok(self.process_credentials(context.caller)?.gid as u64)
    }

    fn syscall_setuid(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_credential_update(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let uid = u16::try_from(context.arg(0)).map_err(|_| KernelError::InvalidArgument)?;
        self.process_credentials_mut(context.caller)?.set_uid(uid);
        Ok(0)
    }

    fn syscall_setgid(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_credential_update(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let gid = u16::try_from(context.arg(0)).map_err(|_| KernelError::InvalidArgument)?;
        self.process_credentials_mut(context.caller)?.set_gid(gid);
        Ok(0)
    }

    fn syscall_getgroups(&self, context: SyscallContext) -> KernelResult<u64> {
        let credentials = self.process_credentials(context.caller)?;
        let group_count = credentials.supplementary_group_count();
        if context.arg(0) == 0 {
            return Ok(group_count as u64);
        }
        let capacity = context.arg(0) as usize;
        if capacity < group_count {
            return Err(KernelError::InvalidArgument);
        }
        let groups = user_slice_mut_typed::<u32>(context.arg(1), capacity)?;
        let stored = credentials.supplementary_groups();
        let mut idx = 0usize;
        while idx < group_count {
            groups[idx] = stored[idx] as u32;
            idx += 1;
        }
        Ok(group_count as u64)
    }

    fn syscall_setgroups(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_credential_update(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let count = context.arg(0) as usize;
        if count > MAX_SUPPLEMENTARY_GROUPS {
            return Err(KernelError::InvalidArgument);
        }
        let user_groups = user_slice_typed::<u32>(context.arg(1), count)?;
        let mut groups = [0u16; MAX_SUPPLEMENTARY_GROUPS];
        let mut idx = 0usize;
        while idx < count {
            groups[idx] =
                u16::try_from(user_groups[idx]).map_err(|_| KernelError::InvalidArgument)?;
            idx += 1;
        }
        self.process_credentials_mut(context.caller)?
            .set_supplementary_groups(&groups[..count])
            .map_err(|_| KernelError::InvalidArgument)?;
        Ok(0)
    }

    fn syscall_rt_sigaction(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let signal = context.arg(0) as usize;
        if signal == 0 || signal > crate::kernel::process::MAX_SIGNAL_NUMBER {
            return Err(KernelError::InvalidArgument);
        }
        let new_action = context.arg(1) as *const SignalAction;
        let old_action = context.arg(2) as *mut SignalAction;
        let index = self.locate_process(context.caller)?;
        let pcb = self.process_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?;
        if !old_action.is_null() {
            unsafe { old_action.write(pcb.signal_actions[signal]) };
        }
        if !new_action.is_null() {
            pcb.signal_actions[signal] = unsafe { new_action.read() };
        }
        Ok(0)
    }

    fn syscall_rt_sigprocmask(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let how = context.arg(0);
        let new_mask = context.arg(1) as *const SignalMask;
        let old_mask = context.arg(2) as *mut SignalMask;
        let thread = context.thread.ok_or(KernelError::UnknownThread)?;
        let index = self.locate_thread(thread)?;
        let tcb = self.thread_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownThread)?;
        if !old_mask.is_null() {
            unsafe { old_mask.write(tcb.signal_mask) };
        }
        if !new_mask.is_null() {
            let requested = unsafe { new_mask.read() };
            let mask = match how {
                0 => SignalMask::from_bits(tcb.signal_mask.bits() | requested.bits()),
                1 => SignalMask::from_bits(tcb.signal_mask.bits() & !requested.bits()),
                2 => requested,
                _ => return Err(KernelError::InvalidArgument),
            };
            tcb.set_signal_mask(mask);
        }
        Ok(0)
    }

    fn syscall_kill(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.send_signal(context.arg(0) as i64, context.arg(1) as u8)
    }

    fn syscall_rt_sigreturn(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let thread = context.thread.ok_or(KernelError::UnknownThread)?;
        let index = self.locate_thread(thread)?;
        self.thread_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownThread)?
            .finish_signal();
        Ok(0)
    }

    fn syscall_clock_gettime(&self, context: SyscallContext) -> KernelResult<u64> {
        let out = user_out_ptr::<MirageTimespec>(context.arg(1))?;
        let nanos = KERNEL_TIME.now().as_nanos();
        unsafe {
            out.write(MirageTimespec {
                tv_sec: (nanos / 1_000_000_000) as i64,
                tv_nsec: (nanos % 1_000_000_000) as i64,
            });
        }
        Ok(0)
    }

    fn syscall_nanosleep(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let requested = read_user_value::<MirageTimespec>(context.arg(0))?;
        if context.arg(1) != 0 {
            let _ = user_out_ptr::<MirageTimespec>(context.arg(1))?;
        }
        let duration_ns = timespec_to_nanos(requested)?;
        if duration_ns == 0 {
            return Ok(0);
        }

        let wake_deadline = KERNEL_TIME.now().as_nanos().saturating_add(duration_ns);
        self.timers
            .add_sleep(context.caller, context.thread, wake_deadline)
            .map_err(map_timer_error)?;
        let process_index = self.locate_process(context.caller)?;
        self.block_process_at_index(context.caller, process_index);

        if context.arg(1) != 0 {
            write_user_value(
                context.arg(1),
                MirageTimespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                },
            )?;
        }

        Ok(0)
    }

    fn syscall_timer_create(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.ensure_process_exists(context.caller)?;
        let timer_id_out = context.arg(2);
        let _ = user_out_ptr::<u64>(timer_id_out)?;
        let id = self
            .timers
            .create_timer(context.caller)
            .map_err(map_timer_error)?;
        write_user_value(timer_id_out, id)?;
        Ok(0)
    }

    fn syscall_timer_settime(&mut self, context: SyscallContext) -> KernelResult<u64> {
        const TIMER_ABSTIME: u64 = 1;

        let timer_id = context.arg(0);
        let flags = context.arg(1);
        if flags & !TIMER_ABSTIME != 0 {
            return Err(KernelError::InvalidArgument);
        }

        let requested = read_user_value::<MirageItimerspec>(context.arg(2))?;
        let old_value_out = context.arg(3);
        if old_value_out != 0 {
            let _ = user_out_ptr::<MirageItimerspec>(old_value_out)?;
        }
        let value_ns = timespec_to_nanos(requested.it_value)?;
        let interval_ns = timespec_to_nanos(requested.it_interval)?;
        let now_ns = KERNEL_TIME.now().as_nanos();
        let deadline = if value_ns == 0 {
            None
        } else if flags & TIMER_ABSTIME != 0 {
            Some(value_ns)
        } else {
            Some(now_ns.saturating_add(value_ns))
        };

        let previous = self
            .timers
            .set_timer(context.caller, timer_id, deadline, interval_ns)
            .map_err(map_timer_error)?;

        if old_value_out != 0 {
            write_user_value(old_value_out, timer_to_itimerspec(previous, now_ns))?;
        }

        Ok(0)
    }

    fn syscall_timer_gettime(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let _ = user_out_ptr::<MirageItimerspec>(context.arg(1))?;
        let timer = self
            .timers
            .timer(context.caller, context.arg(0))
            .map_err(map_timer_error)?;
        write_user_value(
            context.arg(1),
            timer_to_itimerspec(timer, KERNEL_TIME.now().as_nanos()),
        )?;
        Ok(0)
    }

    fn syscall_timer_delete(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.timers
            .delete_timer(context.caller, context.arg(0))
            .map_err(map_timer_error)?;
        Ok(0)
    }

    fn syscall_dup(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.duplicate_fd(
            context.caller,
            context.arg(0) as usize,
            0,
            DescriptorFlags::EMPTY,
        )
    }

    fn syscall_dup2(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.duplicate_fd_to(
            context.caller,
            context.arg(0) as usize,
            context.arg(1) as usize,
            DescriptorFlags::EMPTY,
            false,
        )
    }

    fn syscall_dup3(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let flags = context.arg(2);
        if flags & !O_CLOEXEC_RAW != 0 || context.arg(0) == context.arg(1) {
            return Err(KernelError::InvalidArgument);
        }
        let descriptor_flags = if flags & O_CLOEXEC_RAW != 0 {
            DescriptorFlags::CLOSE_ON_EXEC
        } else {
            DescriptorFlags::EMPTY
        };
        self.duplicate_fd_to(
            context.caller,
            context.arg(0) as usize,
            context.arg(1) as usize,
            descriptor_flags,
            true,
        )
    }

    fn syscall_fcntl(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let fd = context.arg(0) as usize;
        match context.arg(1) {
            F_DUPFD => self.duplicate_fd(
                context.caller,
                fd,
                context.arg(2) as usize,
                DescriptorFlags::EMPTY,
            ),
            F_DUPFD_CLOEXEC => self.duplicate_fd(
                context.caller,
                fd,
                context.arg(2) as usize,
                DescriptorFlags::CLOSE_ON_EXEC,
            ),
            F_GETFD => Ok(self
                .process_files(context.caller)?
                .get(fd)
                .map_err(map_process_file_table_error)?
                .flags()
                .bits() as u64),
            F_SETFD => {
                let flags = if context.arg(2) & FD_CLOEXEC != 0 {
                    DescriptorFlags::CLOSE_ON_EXEC
                } else {
                    DescriptorFlags::EMPTY
                };
                self.process_files_mut(context.caller)?
                    .get_mut(fd)
                    .map_err(map_process_file_table_error)?
                    .set_flags(flags);
                Ok(0)
            }
            F_GETFL => {
                let description = self.fd_description(context.caller, fd)?;
                Ok(self
                    .open_files
                    .get(description)
                    .map_err(map_file_table_error)?
                    .flags()
                    .bits() as u64)
            }
            _ => Err(KernelError::InvalidArgument),
        }
    }

    fn syscall_ioctl(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let fd = context.arg(0) as usize;
        let request = context.arg(1);
        let arg = context.arg(2);
        let object = self.fd_object(context.caller, fd)?;
        match request {
            FIONREAD => {
                let out = user_out_ptr::<u32>(arg)?;
                let available = match object {
                    DescriptorObject::Regular(file) => {
                        let stat = self.root_fs.fstat(&file).map_err(KernelError::Filesystem)?;
                        stat.size.saturating_sub(file.cursor()).min(u32::MAX as u64) as u32
                    }
                    DescriptorObject::Pipe(endpoint) => self
                        .pipes
                        .get(endpoint.id().raw())
                        .and_then(|entry| *entry)
                        .map(|pipe| pipe.len as u32)
                        .ok_or(KernelError::InvalidArgument)?,
                    DescriptorObject::EventFd(id) => self
                        .eventfds
                        .get(id.raw())
                        .and_then(|entry| *entry)
                        .map(|eventfd| if eventfd.counter > 0 { 8 } else { 0 })
                        .ok_or(KernelError::InvalidArgument)?,
                    DescriptorObject::Device(_) | DescriptorObject::Socket(_) => 0,
                };
                unsafe { out.write(available) };
                Ok(0)
            }
            BLKSSZGET => match object {
                DescriptorObject::Device(handle) => {
                    let out = user_out_ptr::<u32>(arg)?;
                    let value = self
                        .devices
                        .sector_size(handle.id())
                        .map_err(KernelError::DeviceFault)? as u32;
                    unsafe { out.write(value) };
                    Ok(0)
                }
                _ => Err(KernelError::InvalidArgument),
            },
            BLKGETSIZE64 => match object {
                DescriptorObject::Device(handle) => {
                    let out = user_out_ptr::<u64>(arg)?;
                    let size = self
                        .devices
                        .sector_size(handle.id())
                        .and_then(|sector_size| {
                            self.devices
                                .sector_count(handle.id())
                                .map(|sectors| sectors.saturating_mul(sector_size as u64))
                        })
                        .map_err(KernelError::DeviceFault)?;
                    unsafe { out.write(size) };
                    Ok(0)
                }
                _ => Err(KernelError::InvalidArgument),
            },
            MIRAGE_IOCTL_DEVICE_INFO => match object {
                DescriptorObject::Device(handle) => {
                    let out = user_out_ptr::<MirageDeviceDescriptor>(arg)?;
                    let descriptor = self
                        .device_info(handle.id())
                        .ok_or(KernelError::DeviceNotFound)?;
                    unsafe { out.write(MirageDeviceDescriptor::from_descriptor(descriptor)) };
                    Ok(0)
                }
                _ => Err(KernelError::InvalidArgument),
            },
            _ => Err(KernelError::InvalidArgument),
        }
    }

    fn syscall_pipe2(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let out = user_slice_mut_typed::<i32>(context.arg(0), 2)?;
        let flags = context.arg(1);
        if flags & !PIPE2_SUPPORTED_FLAGS != 0 {
            return Err(KernelError::InvalidArgument);
        }
        let pipe_id = self.allocate_pipe()?;
        let descriptor_flags = descriptor_flags_from_raw(flags);
        let read_description = self
            .open_files
            .insert_object(DescriptorObject::Pipe(PipeEndpoint::new(
                pipe_id,
                PipeDirection::Read,
            )))
            .map_err(map_file_table_error)?;
        let write_description =
            match self
                .open_files
                .insert_object(DescriptorObject::Pipe(PipeEndpoint::new(
                    pipe_id,
                    PipeDirection::Write,
                ))) {
                Ok(description) => description,
                Err(error) => {
                    let _ = self.close_open_description(read_description);
                    self.pipes[pipe_id.raw()] = None;
                    return Err(map_file_table_error(error));
                }
            };
        let read_fd = match self
            .process_files_mut(context.caller)?
            .open(read_description, descriptor_flags)
        {
            Ok(fd) => fd,
            Err(error) => {
                let _ = self.close_open_description(read_description);
                let _ = self.close_open_description(write_description);
                self.pipes[pipe_id.raw()] = None;
                return Err(map_process_file_table_error(error));
            }
        };
        let write_fd = match self
            .process_files_mut(context.caller)?
            .open(write_description, descriptor_flags)
        {
            Ok(fd) => fd,
            Err(error) => {
                let closed_read = self
                    .process_files_mut(context.caller)
                    .ok()
                    .and_then(|files| files.close(read_fd).ok());
                if let Some(descriptor) = closed_read {
                    let _ = self.close_open_description(descriptor.description());
                }
                let _ = self.close_open_description(write_description);
                self.pipes[pipe_id.raw()] = None;
                return Err(map_process_file_table_error(error));
            }
        };
        out[0] = read_fd as i32;
        out[1] = write_fd as i32;
        Ok(0)
    }

    fn syscall_poll(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let fds = user_slice_mut_typed::<MiragePollFd>(context.arg(0), context.arg(1) as usize)?;
        let timeout_ms = context.arg(2) as i32;
        self.poll_descriptors(context.caller, context.thread, fds, timeout_ms)
    }

    fn syscall_pselect(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let fds = user_slice_mut_typed::<MiragePollFd>(context.arg(0), context.arg(1) as usize)?;
        let timeout_ms = if context.arg(2) == 0 {
            -1
        } else {
            let timeout = read_user_value::<MirageTimespec>(context.arg(2))?;
            let nanos = timespec_to_nanos(timeout)?;
            let millis = (nanos / 1_000_000).min(i32::MAX as u128) as i32;
            millis
        };
        self.poll_descriptors(context.caller, context.thread, fds, timeout_ms)
    }

    fn syscall_eventfd(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let initial = context.arg(0) as u32 as u64;
        let flags = context.arg(1);
        if flags & !EVENTFD_SUPPORTED_FLAGS != 0 || flags & O_DIRECT_RAW != 0 {
            return Err(KernelError::InvalidArgument);
        }
        let id = self.allocate_eventfd(initial, flags & EFD_SEMAPHORE != 0)?;
        let description = match self.open_files.insert_object(DescriptorObject::EventFd(id)) {
            Ok(description) => description,
            Err(error) => {
                self.eventfds[id.raw()] = None;
                return Err(map_file_table_error(error));
            }
        };
        match self
            .process_files_mut(context.caller)?
            .open(description, descriptor_flags_from_raw(flags))
        {
            Ok(fd) => Ok(fd as u64),
            Err(error) => {
                let _ = self.close_open_description(description);
                self.eventfds[id.raw()] = None;
                Err(map_process_file_table_error(error))
            }
        }
    }

    fn syscall_socket(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let domain = i32::try_from(context.arg(0)).map_err(|_| KernelError::InvalidArgument)?;
        let socket_type =
            i32::try_from(context.arg(1)).map_err(|_| KernelError::InvalidArgument)?;
        let protocol = i32::try_from(context.arg(2)).map_err(|_| KernelError::InvalidArgument)?;
        if domain < 0 || socket_type < 0 {
            return Err(KernelError::InvalidArgument);
        }

        let handle = self.allocate_socket_handle()?;
        let description = match self
            .open_files
            .insert_object(DescriptorObject::Socket(handle))
        {
            Ok(description) => description,
            Err(error) => return Err(map_file_table_error(error)),
        };
        let fd = match self
            .process_files_mut(context.caller)?
            .open(description, DescriptorFlags::EMPTY)
        {
            Ok(fd) => fd,
            Err(error) => {
                let _ = self.close_open_description(description);
                return Err(map_process_file_table_error(error));
            }
        };

        let request = NetworkSocketRequest {
            header: self.network_request_header(
                NetworkOpcode::Socket,
                context.caller,
                socket_type as u32,
            ),
            socket_handle: handle.raw(),
            domain,
            socket_type,
            protocol,
            reserved: 0,
        };
        if let Err(error) = self.send_network_request(context.caller, &request) {
            if let Ok(descriptor) = self
                .process_files_mut(context.caller)
                .and_then(|files| files.close(fd).map_err(map_process_file_table_error))
            {
                let _ = self.close_open_description(descriptor.description());
            }
            return Err(error);
        }

        Ok(fd as u64)
    }

    fn syscall_bind(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let handle = self.socket_handle_for_fd(context.caller, context.arg(0) as usize)?;
        let addr_ptr = context.arg(1);
        let addr_len = u32::try_from(context.arg(2)).map_err(|_| KernelError::InvalidArgument)?;
        validate_user_range(addr_ptr, addr_len as usize)?;
        let request = NetworkSockaddrRequest {
            header: self.network_request_header(NetworkOpcode::Bind, context.caller, 0),
            socket_handle: handle.raw(),
            addr_ptr,
            addr_len,
            value: 0,
            result_addr_len_ptr: 0,
            accepted_socket_handle: 0,
        };
        self.send_network_request(context.caller, &request)?;
        Ok(0)
    }

    fn syscall_listen(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let handle = self.socket_handle_for_fd(context.caller, context.arg(0) as usize)?;
        let backlog = i32::try_from(context.arg(1)).map_err(|_| KernelError::InvalidArgument)?;
        if backlog < 0 {
            return Err(KernelError::InvalidArgument);
        }
        let request = NetworkSockaddrRequest {
            header: self.network_request_header(NetworkOpcode::Listen, context.caller, 0),
            socket_handle: handle.raw(),
            addr_ptr: 0,
            addr_len: 0,
            value: backlog,
            result_addr_len_ptr: 0,
            accepted_socket_handle: 0,
        };
        self.send_network_request(context.caller, &request)?;
        Ok(0)
    }

    fn syscall_accept(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let listener = self.socket_handle_for_fd(context.caller, context.arg(0) as usize)?;
        let addr_ptr = context.arg(1);
        let addr_len_ptr = context.arg(2);
        if addr_ptr != 0 {
            if addr_len_ptr == 0 {
                return Err(KernelError::InvalidPointer);
            }
            let addr_len = read_user_value::<u32>(addr_len_ptr)?;
            validate_user_range(addr_ptr, addr_len as usize)?;
        } else if addr_len_ptr != 0 {
            let _ = read_user_value::<u32>(addr_len_ptr)?;
        }
        let accepted = self.allocate_socket_handle()?;
        let description = self
            .open_files
            .insert_object(DescriptorObject::Socket(accepted))
            .map_err(map_file_table_error)?;
        let fd = match self
            .process_files_mut(context.caller)?
            .open(description, DescriptorFlags::EMPTY)
        {
            Ok(fd) => fd,
            Err(error) => {
                let _ = self.close_open_description(description);
                return Err(map_process_file_table_error(error));
            }
        };
        let request = NetworkSockaddrRequest {
            header: self.network_request_header(NetworkOpcode::Accept, context.caller, 0),
            socket_handle: listener.raw(),
            addr_ptr,
            addr_len: 0,
            value: 0,
            result_addr_len_ptr: addr_len_ptr,
            accepted_socket_handle: accepted.raw(),
        };
        if let Err(error) = self.send_network_request(context.caller, &request) {
            if let Ok(descriptor) = self
                .process_files_mut(context.caller)
                .and_then(|files| files.close(fd).map_err(map_process_file_table_error))
            {
                let _ = self.close_open_description(descriptor.description());
            }
            return Err(error);
        }

        Ok(fd as u64)
    }

    fn syscall_connect(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let handle = self.socket_handle_for_fd(context.caller, context.arg(0) as usize)?;
        let addr_ptr = context.arg(1);
        let addr_len = u32::try_from(context.arg(2)).map_err(|_| KernelError::InvalidArgument)?;
        validate_user_range(addr_ptr, addr_len as usize)?;
        let request = NetworkSockaddrRequest {
            header: self.network_request_header(NetworkOpcode::Connect, context.caller, 0),
            socket_handle: handle.raw(),
            addr_ptr,
            addr_len,
            value: 0,
            result_addr_len_ptr: 0,
            accepted_socket_handle: 0,
        };
        self.send_network_request(context.caller, &request)?;
        Ok(0)
    }

    fn syscall_sendmsg(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let handle = self.socket_handle_for_fd(context.caller, context.arg(0) as usize)?;
        let message_ptr = context.arg(1);
        validate_user_range(message_ptr, 1)?;
        let request = NetworkSendmsgRequest {
            header: self.network_request_header(NetworkOpcode::Sendmsg, context.caller, 0),
            socket_handle: handle.raw(),
            message_ptr,
            flags: context.arg(2),
            reserved: 0,
        };
        self.send_network_request(context.caller, &request)?;
        Ok(0)
    }

    fn syscall_recvmsg(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let handle = self.socket_handle_for_fd(context.caller, context.arg(0) as usize)?;
        let message_ptr = context.arg(1);
        validate_user_range(message_ptr, 1)?;
        let request = NetworkRecvmsgRequest {
            header: self.network_request_header(NetworkOpcode::Recvmsg, context.caller, 0),
            socket_handle: handle.raw(),
            message_ptr,
            flags: context.arg(2),
            reserved: 0,
        };
        self.send_network_request(context.caller, &request)?;
        Ok(0)
    }

    fn syscall_clone(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let entry_point = context.arg(0);
        let priority = decode_priority(context.arg(1))?;
        let flags = context.arg(2);
        let request = if flags == 0 && context.arg(3) == 0 && context.arg(4) == 0 {
            CloneTaskRequest::legacy_thread(context.caller, context.thread, entry_point, priority)
        } else {
            let tls_base = if context.arg(3) == 0 {
                None
            } else {
                Some(context.arg(3))
            };
            let child_stack = if context.arg(4) == 0 {
                None
            } else {
                Some(context.arg(4))
            };
            CloneTaskRequest::new(
                context.caller,
                context.thread,
                entry_point,
                priority,
                child_stack,
                tls_base,
                flags,
            )
        };
        self.clone_thread(request).map(|thread| thread.raw())
    }

    fn syscall_futex(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let uaddr = context.arg(0);
        let operation = context.arg(1);
        let value = context.arg(2) as i32;
        let timeout_ptr = context.arg(3);
        let command = operation & FUTEX_CMD_MASK;
        let key = self.futex_key(context.caller, uaddr)?;

        match command {
            FUTEX_WAIT => {
                let thread = context.thread.ok_or(KernelError::UnknownThread)?;
                let observed = read_user_value::<i32>(uaddr)?;
                if observed != value {
                    return Err(KernelError::MessageQueueEmpty);
                }
                let deadline = if timeout_ptr == 0 {
                    None
                } else {
                    let requested = read_user_value::<MirageTimespec>(timeout_ptr)?;
                    let duration_ns = timespec_to_nanos(requested)?;
                    Some(KERNEL_TIME.now().as_nanos().saturating_add(duration_ns))
                };
                self.futexes
                    .enqueue(key, thread, deadline)
                    .map_err(|_| KernelError::AllocationFailed)?;
                self.block_thread(thread)?;
                Ok(0)
            }
            FUTEX_WAKE => {
                let limit = if value < 0 { 0 } else { value as usize };
                let mut woken_threads = [None; MAX_THREADS];
                let count = self.futexes.wake(key, limit, &mut woken_threads);
                self.wake_futex_threads(&woken_threads, count, 0)?;
                Ok(count as u64)
            }
            _ => Err(KernelError::InvalidArgument),
        }
    }

    fn syscall_set_thread_area(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let thread = context.thread.ok_or(KernelError::UnknownThread)?;
        let base = context.arg(0);
        validate_tls_base(base)?;
        self.set_thread_fs_base(thread, base)
    }

    fn syscall_arch_prctl(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let thread = context.thread.ok_or(KernelError::UnknownThread)?;
        match context.arg(0) {
            ARCH_SET_FS => {
                let base = context.arg(1);
                validate_tls_base(base)?;
                self.set_thread_fs_base(thread, base)
            }
            ARCH_SET_GS => {
                let base = context.arg(1);
                validate_tls_base(base)?;
                self.set_thread_gs_base(thread, base)
            }
            ARCH_GET_FS => {
                let base = self.thread_fs_base(thread)?;
                write_user_value::<u64>(context.arg(1), base)?;
                Ok(0)
            }
            ARCH_GET_GS => {
                let base = self.thread_gs_base(thread)?;
                write_user_value::<u64>(context.arg(1), base)?;
                Ok(0)
            }
            _ => Err(KernelError::InvalidArgument),
        }
    }

    fn syscall_spawn(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let entry_point = context.arg(0);
        let priority = decode_priority(context.arg(1))?;
        let credentials = decode_credentials(context.arg(2))?;
        self.spawn_task(SpawnTaskRequest {
            parent: Some(context.caller),
            entry_point,
            priority,
            credentials,
        })
        .map(|pid| pid.raw())
    }

    fn syscall_send_ipc(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let receiver = ProcessId::new(context.arg(0));
        let data_ptr = context.arg(1) as *const u8;
        let data_len = context.arg(2) as usize;
        let security_class = decode_security_class(context.arg(3))?;
        if data_len > 0 && data_ptr.is_null() {
            return Err(KernelError::InvalidPointer);
        }

        let data = if data_len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(data_ptr, data_len) }
        };
        let payload = MessagePayload::from_slice(security_class, data);
        self.send_message(context.caller, receiver, payload)?;
        Ok(payload.length as u64)
    }

    fn syscall_receive_ipc(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_ipc_receive(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let out = context.arg(0) as *mut Message;
        if out.is_null() {
            return Err(KernelError::InvalidPointer);
        }
        let message = self.receive_message(context.caller)?;
        unsafe { out.write(message) };
        Ok(message.payload.length as u64)
    }

    fn syscall_register_service(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let service = decode_registry_service_id(context.arg(0))?;
        let owner = if context.arg(1) == 0 {
            context.caller
        } else {
            ProcessId::new(context.arg(1))
        };
        self.register_service(context.caller, service, owner)?;
        Ok(0)
    }

    fn syscall_send_service_ipc(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let service = decode_registry_service_id(context.arg(0))?;
        let data_ptr = context.arg(1) as *const u8;
        let data_len = context.arg(2) as usize;
        let security_class = decode_security_class(context.arg(3))?;
        if data_len > 0 && data_ptr.is_null() {
            return Err(KernelError::InvalidPointer);
        }
        let data = if data_len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(data_ptr, data_len) }
        };
        let payload = MessagePayload::from_slice(security_class, data);
        self.send_service_message(context.caller, service, payload)?;
        Ok(payload.length as u64)
    }

    fn syscall_claim_device(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let service = decode_registry_service_id(context.arg(0))?;
        let device = DeviceId::new(context.arg(1) as u16);
        self.claim_service_device(context.caller, service, device)?;
        Ok(0)
    }

    fn syscall_release_device(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let service = decode_registry_service_id(context.arg(0))?;
        let device = DeviceId::new(context.arg(1) as u16);
        self.release_service_device(context.caller, service, device)?;
        Ok(0)
    }

    fn syscall_receive_or_block_ipc(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_ipc_receive(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let out = context.arg(0) as *mut Message;
        if out.is_null() {
            return Err(KernelError::InvalidPointer);
        }

        if let Some(message) = self.receive_or_block(context.caller)? {
            unsafe { out.write(message) };
            Ok(1)
        } else {
            Ok(0)
        }
    }

    fn syscall_enumerate_devices(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_device_enumeration(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let out = context.arg(0) as *mut MirageDeviceDescriptor;
        let capacity = context.arg(1) as usize;
        if capacity > 0 && out.is_null() {
            return Err(KernelError::InvalidPointer);
        }
        let out_slice = if capacity == 0 {
            &mut []
        } else {
            unsafe { core::slice::from_raw_parts_mut(out, capacity) }
        };

        let mut descriptors = [EMPTY_DEVICE_DESCRIPTOR; MAX_DEVICES];
        let count = self.enumerate_devices(&mut descriptors[..min(capacity, MAX_DEVICES)]);
        let mut idx = 0usize;
        while idx < count {
            out_slice[idx] = MirageDeviceDescriptor::from_descriptor(descriptors[idx]);
            idx += 1;
        }
        Ok(count as u64)
    }

    fn syscall_device_info(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_device_enumeration(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let id = DeviceId::new(context.arg(0) as u16);
        let out = context.arg(1) as *mut MirageDeviceDescriptor;
        if out.is_null() {
            return Err(KernelError::InvalidPointer);
        }
        let descriptor = self.device_info(id).ok_or(KernelError::DeviceNotFound)?;
        unsafe { out.write(MirageDeviceDescriptor::from_descriptor(descriptor)) };
        Ok(1)
    }

    fn syscall_device_read(&self, context: SyscallContext) -> KernelResult<u64> {
        let id = DeviceId::new(context.arg(0) as u16);
        let buffer = context.arg(1) as *mut u8;
        let len = context.arg(2) as usize;
        if len > 0 && buffer.is_null() {
            return Err(KernelError::InvalidPointer);
        }
        let buffer = if len == 0 {
            &mut []
        } else {
            unsafe { core::slice::from_raw_parts_mut(buffer, len) }
        };
        self.device_read(context.caller, id, buffer)
            .map(|read| read as u64)
    }

    fn syscall_device_write(&self, context: SyscallContext) -> KernelResult<u64> {
        let id = DeviceId::new(context.arg(0) as u16);
        let data = context.arg(1) as *const u8;
        let len = context.arg(2) as usize;
        if len > 0 && data.is_null() {
            return Err(KernelError::InvalidPointer);
        }
        let data = if len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(data, len) }
        };
        self.device_write(context.caller, id, data)
            .map(|written| written as u64)
    }

    fn syscall_mmap(&self, context: SyscallContext) -> KernelResult<u64> {
        let length = context.arg(0) as usize;
        let protection = MemoryProtection::from_bits(context.arg(1) as u32);
        self.security
            .authorize_memory_mapping(context.caller, protection)
            .map_err(KernelError::SecurityViolation)?;
        memory::mmap_for(context.caller, length, protection)
            .map(|region| region.as_ptr() as u64)
            .ok_or(KernelError::AllocationFailed)
    }

    fn syscall_munmap(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_memory_service(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let ptr = NonNull::new(context.arg(0) as *mut u8).ok_or(KernelError::InvalidPointer)?;
        let length = context.arg(1) as usize;
        if memory::munmap_ptr_for(context.caller, ptr, length) {
            Ok(0)
        } else {
            Err(KernelError::InvalidArgument)
        }
    }

    fn syscall_malloc(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_memory_service(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        memory::malloc_for(context.caller, context.arg(0) as usize)
            .map(|ptr| ptr.as_ptr() as u64)
            .ok_or(KernelError::AllocationFailed)
    }

    fn syscall_free(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_memory_service(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let ptr = NonNull::new(context.arg(0) as *mut u8).ok_or(KernelError::InvalidPointer)?;
        if memory::free_for(context.caller, ptr) {
            Ok(0)
        } else {
            Err(KernelError::InvalidArgument)
        }
    }

    fn syscall_realloc(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_memory_service(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let ptr = NonNull::new(context.arg(0) as *mut u8);
        memory::realloc_for(context.caller, ptr, context.arg(1) as usize)
            .map(|ptr| ptr.as_ptr() as u64)
            .ok_or(KernelError::AllocationFailed)
    }

    fn syscall_malloc_aligned(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_memory_service(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let alignment = context.arg(1) as usize;
        if alignment == 0 || !alignment.is_power_of_two() {
            return Err(KernelError::InvalidArgument);
        }

        memory::malloc_aligned_for(context.caller, context.arg(0) as usize, alignment)
            .map(|ptr| ptr.as_ptr() as u64)
            .ok_or(KernelError::AllocationFailed)
    }

    fn syscall_openat(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let path = path_buf.as_path()?;
        let raw_flags = open_flags_from_libc(context.arg(2) as u32);
        let descriptor_flags = DescriptorFlags::from_open_flags(raw_flags);
        let credentials = self.fs_credentials_for(context.caller)?;
        let file = self
            .root_fs
            .open(path, raw_flags.without_descriptor_flags(), credentials)
            .map_err(KernelError::Filesystem)?;
        let description = self.open_files.insert(file).map_err(map_file_table_error)?;
        match self
            .process_files_mut(context.caller)?
            .open(description, descriptor_flags)
        {
            Ok(fd) => Ok(fd as u64),
            Err(error) => {
                self.close_open_description(description)?;
                Err(map_process_file_table_error(error))
            }
        }
    }

    fn syscall_close(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let descriptor = self
            .process_files_mut(context.caller)?
            .close(context.arg(0) as usize)
            .map_err(map_process_file_table_error)?;
        self.close_open_description(descriptor.description())?;
        Ok(0)
    }

    fn syscall_read(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let buffer = user_slice_mut(context.arg(1), context.arg(2) as usize)?;
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        match self
            .open_files
            .get_object(description)
            .map_err(map_file_table_error)?
        {
            DescriptorObject::Regular(_) => {
                let file = self
                    .open_files
                    .get_mut(description)
                    .map_err(map_file_table_error)?;
                self.root_fs
                    .read(file, buffer)
                    .map(|read| read as u64)
                    .map_err(KernelError::Filesystem)
            }
            DescriptorObject::Pipe(endpoint) => {
                if endpoint.direction() != PipeDirection::Read {
                    return Err(KernelError::InvalidArgument);
                }
                let pipe = self
                    .pipes
                    .get_mut(endpoint.id().raw())
                    .and_then(Option::as_mut)
                    .ok_or(KernelError::InvalidArgument)?;
                Ok(pipe.read(buffer) as u64)
            }
            DescriptorObject::EventFd(id) => {
                if buffer.len() < 8 {
                    return Err(KernelError::InvalidArgument);
                }
                let eventfd = self
                    .eventfds
                    .get_mut(id.raw())
                    .and_then(Option::as_mut)
                    .ok_or(KernelError::InvalidArgument)?;
                if eventfd.counter == 0 {
                    return Ok(0);
                }
                let value = if eventfd.semaphore {
                    eventfd.counter -= 1;
                    1
                } else {
                    let value = eventfd.counter;
                    eventfd.counter = 0;
                    value
                };
                buffer[..8].copy_from_slice(&value.to_ne_bytes());
                Ok(8)
            }
            DescriptorObject::Device(handle) => self
                .device_read(context.caller, handle.id(), buffer)
                .map(|read| read as u64),
            DescriptorObject::Socket(_) => Err(KernelError::InvalidArgument),
        }
    }

    fn syscall_write(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let data = user_slice(context.arg(1), context.arg(2) as usize)?;
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        match self
            .open_files
            .get_object(description)
            .map_err(map_file_table_error)?
        {
            DescriptorObject::Regular(_) => {
                let file = self
                    .open_files
                    .get_mut(description)
                    .map_err(map_file_table_error)?;
                self.root_fs
                    .write(file, data)
                    .map(|written| written as u64)
                    .map_err(KernelError::Filesystem)
            }
            DescriptorObject::Pipe(endpoint) => {
                if endpoint.direction() != PipeDirection::Write {
                    return Err(KernelError::InvalidArgument);
                }
                let pipe = self
                    .pipes
                    .get_mut(endpoint.id().raw())
                    .and_then(Option::as_mut)
                    .ok_or(KernelError::InvalidArgument)?;
                if pipe.readers == 0 {
                    return Err(KernelError::InvalidArgument);
                }
                Ok(pipe.write(data) as u64)
            }
            DescriptorObject::EventFd(id) => {
                if data.len() < 8 {
                    return Err(KernelError::InvalidArgument);
                }
                let value = u64::from_ne_bytes([
                    data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
                ]);
                if value == u64::MAX {
                    return Err(KernelError::InvalidArgument);
                }
                let eventfd = self
                    .eventfds
                    .get_mut(id.raw())
                    .and_then(Option::as_mut)
                    .ok_or(KernelError::InvalidArgument)?;
                eventfd.counter = eventfd
                    .counter
                    .checked_add(value)
                    .ok_or(KernelError::InvalidArgument)?;
                Ok(8)
            }
            DescriptorObject::Device(handle) => self
                .device_write(context.caller, handle.id(), data)
                .map(|written| written as u64),
            DescriptorObject::Socket(_) => Err(KernelError::InvalidArgument),
        }
    }

    fn syscall_pread64(&self, context: SyscallContext) -> KernelResult<u64> {
        let buffer = user_slice_mut(context.arg(1), context.arg(2) as usize)?;
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        self.root_fs
            .pread(&file, buffer, context.arg(3))
            .map(|read| read as u64)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_pwrite64(&self, context: SyscallContext) -> KernelResult<u64> {
        let data = user_slice(context.arg(1), context.arg(2) as usize)?;
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        self.root_fs
            .pwrite(&file, data, context.arg(3))
            .map(|written| written as u64)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_lseek(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let fd = context.arg(0) as usize;
        let offset = context.arg(1) as i64;
        let whence = context.arg(2);
        let description = self.fd_description(context.caller, fd)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        let current = file.cursor() as i64;
        let end = self
            .root_fs
            .fstat(&file)
            .map_err(KernelError::Filesystem)?
            .size as i64;
        let base = match whence {
            SEEK_SET => 0,
            SEEK_CUR => current,
            SEEK_END => end,
            _ => return Err(KernelError::InvalidArgument),
        };
        let new_offset = base
            .checked_add(offset)
            .ok_or(KernelError::InvalidArgument)?;
        if new_offset < 0 {
            return Err(KernelError::InvalidArgument);
        }
        self.open_files
            .get_mut(description)
            .map_err(map_file_table_error)?
            .seek(new_offset as u64);
        Ok(new_offset as u64)
    }

    fn syscall_statx(&self, context: SyscallContext) -> KernelResult<u64> {
        self.syscall_newfstatat(context)
    }

    fn syscall_newfstatat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_ptr = context.arg(1);
        let out = user_out_ptr::<CStat>(context.arg(2))?;
        let stat = if path_ptr == 0 {
            let description = self.fd_description(context.caller, context.arg(0) as usize)?;
            let file = self
                .open_files
                .get(description)
                .map_err(map_file_table_error)?;
            self.root_fs.fstat(&file)
        } else {
            let path_buf = self.resolve_user_path(context.caller, context.arg(0), path_ptr)?;
            self.root_fs.stat(path_buf.as_path()?)
        }
        .map_err(KernelError::Filesystem)?;
        unsafe { out.write(CStat::from_kernel(stat)) };
        Ok(0)
    }

    fn syscall_getdents64(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let entries = user_slice_mut_typed::<CDirEntry>(context.arg(1), context.arg(2) as usize)?;
        let fd = context.arg(0) as usize;
        let description = self.fd_description(context.caller, fd)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        let mut cursor = file.cursor() as usize;
        let mut written = 0usize;
        while written < entries.len() {
            let mut kernel_entry = [DirEntry::empty(); 1];
            let count = self
                .root_fs
                .readdir_inode(file.inode(), cursor, &mut kernel_entry)
                .map_err(KernelError::Filesystem)?;
            if count == 0 {
                break;
            }
            cursor += count;
            entries[written] = CDirEntry::from_kernel(&kernel_entry[0], cursor);
            written += 1;
        }
        self.open_files
            .get_mut(description)
            .map_err(map_file_table_error)?
            .advance(written);
        Ok(written as u64)
    }

    fn syscall_mkdirat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let path = path_buf.as_path()?;
        let requested = context.arg(2) as u32;
        let umask = self.process_files(context.caller)?.umask().bits() as u32;
        let credentials = self.fs_credentials_for(context.caller)?;
        let mode = permissions_from_libc_mode(requested & !umask, credentials.uid, credentials.gid);
        self.root_fs
            .mkdir(path, mode, credentials)
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_unlinkat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let path = path_buf.as_path()?;
        let flags = context.arg(2);
        let result = if (flags & AT_REMOVEDIR) != 0 {
            self.root_fs
                .rmdir(path, self.fs_credentials_for(context.caller)?)
        } else {
            self.root_fs
                .unlink(path, self.fs_credentials_for(context.caller)?)
        };
        result.map(|_| 0).map_err(KernelError::Filesystem)
    }

    fn syscall_renameat2(&self, context: SyscallContext) -> KernelResult<u64> {
        let old_path_buf =
            self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let new_path_buf =
            self.resolve_user_path(context.caller, context.arg(2), context.arg(3))?;
        let flags = context.arg(4);
        if flags & !RENAME_NOREPLACE != 0 {
            return Err(KernelError::InvalidArgument);
        }
        if flags & RENAME_NOREPLACE != 0 && self.root_fs.stat(new_path_buf.as_path()?).is_ok() {
            return Err(KernelError::Filesystem(VfsError::AlreadyExists));
        }
        self.root_fs
            .rename(
                old_path_buf.as_path()?,
                new_path_buf.as_path()?,
                self.fs_credentials_for(context.caller)?,
            )
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_ftruncate(&self, context: SyscallContext) -> KernelResult<u64> {
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        self.root_fs
            .ftruncate(
                &file,
                context.arg(1),
                self.fs_credentials_for(context.caller)?,
            )
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_fsync(&self, context: SyscallContext) -> KernelResult<u64> {
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        self.root_fs
            .fsync(&file)
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_mount(&self, context: SyscallContext) -> KernelResult<u64> {
        if context.arg(0) != 0 {
            let _source = user_cstr(context.arg(0))?;
        }
        let target = self.user_path(context.arg(1))?;
        let filesystem_type = if context.arg(2) == 0 {
            DEFAULT_ROOT_FILESYSTEM
        } else {
            user_cstr(context.arg(2))?
        };
        if !target.is_root() || !is_supported_root_filesystem(filesystem_type) {
            return Err(KernelError::Filesystem(VfsError::Unsupported));
        }
        Ok(0)
    }

    fn syscall_chdir(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, AT_FDCWD as u64, context.arg(0))?;
        let stat = self
            .root_fs
            .stat(path_buf.as_path()?)
            .map_err(KernelError::Filesystem)?;
        if stat.kind != crate::kernel::fs::inode::InodeKind::Directory {
            return Err(KernelError::Filesystem(VfsError::NotDirectory));
        }
        self.process_files_mut(context.caller)?
            .set_cwd(ProcessPath::from_path(path_buf.as_path()?));
        Ok(0)
    }

    fn syscall_fchdir(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        let stat = self.root_fs.fstat(&file).map_err(KernelError::Filesystem)?;
        if stat.kind != crate::kernel::fs::inode::InodeKind::Directory {
            return Err(KernelError::Filesystem(VfsError::NotDirectory));
        }
        let path = Path::new(file.path()).map_err(map_path_error)?;
        self.process_files_mut(context.caller)?
            .set_cwd(ProcessPath::from_path(path));
        Ok(0)
    }

    fn syscall_getcwd(&self, context: SyscallContext) -> KernelResult<u64> {
        let buffer = user_slice_mut(context.arg(0), context.arg(1) as usize)?;
        let cwd = self.process_files(context.caller)?.cwd();
        let raw = cwd.as_str().as_bytes();
        if buffer.len() < raw.len() + 1 {
            return Err(KernelError::InvalidArgument);
        }
        buffer[..raw.len()].copy_from_slice(raw);
        buffer[raw.len()] = 0;
        Ok(context.arg(0))
    }

    fn syscall_faccessat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let mode = context.arg(2) as u32;
        if mode
            & !(crate::kernel::fs::stdlib::R_OK
                | crate::kernel::fs::stdlib::W_OK
                | crate::kernel::fs::stdlib::X_OK)
            != 0
        {
            return Err(KernelError::InvalidArgument);
        }
        let stat = self
            .root_fs
            .stat(path_buf.as_path()?)
            .map_err(KernelError::Filesystem)?;
        if mode == crate::kernel::fs::stdlib::F_OK {
            return Ok(0);
        }
        let permissions = crate::kernel::fs::Permissions::new(stat.mode, stat.uid, stat.gid);
        let credentials = self.fs_credentials_for(context.caller)?;
        let allowed = match (
            mode & crate::kernel::fs::stdlib::R_OK != 0,
            mode & crate::kernel::fs::stdlib::W_OK != 0,
            mode & crate::kernel::fs::stdlib::X_OK != 0,
        ) {
            (true, true, true) => {
                permissions.allows(credentials, AccessMode::ReadWrite)
                    && permissions.allows(credentials, AccessMode::Execute)
            }
            (true, true, false) => permissions.allows(credentials, AccessMode::ReadWrite),
            (true, false, false) => permissions.allows(credentials, AccessMode::Read),
            (false, true, false) => permissions.allows(credentials, AccessMode::Write),
            (false, false, true) => permissions.allows(credentials, AccessMode::Execute),
            (true, false, true) => {
                permissions.allows(credentials, AccessMode::Read)
                    && permissions.allows(credentials, AccessMode::Execute)
            }
            (false, true, true) => {
                permissions.allows(credentials, AccessMode::Write)
                    && permissions.allows(credentials, AccessMode::Execute)
            }
            _ => true,
        };
        if allowed {
            Ok(0)
        } else {
            Err(KernelError::Filesystem(VfsError::PermissionDenied))
        }
    }

    fn syscall_fchmodat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        self.root_fs
            .chmod(
                path_buf.as_path()?,
                context.arg(2) as u16,
                self.fs_credentials_for(context.caller)?,
            )
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_fchownat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        self.root_fs
            .chown(
                path_buf.as_path()?,
                context.arg(2) as u16,
                context.arg(3) as u16,
                self.fs_credentials_for(context.caller)?,
            )
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_symlinkat(&self, context: SyscallContext) -> KernelResult<u64> {
        let target = self.user_path_str(context.arg(0))?;
        Path::new_unchecked_rooted(target).map_err(map_path_error)?;
        let link_path_buf =
            self.resolve_user_path(context.caller, context.arg(1), context.arg(2))?;
        self.root_fs
            .symlink(
                target,
                link_path_buf.as_path()?,
                self.fs_credentials_for(context.caller)?,
            )
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_readlinkat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let buffer = user_slice_mut(context.arg(2), context.arg(3) as usize)?;
        self.root_fs
            .readlink(path_buf.as_path()?, buffer)
            .map(|n| n as u64)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_linkat(&self, context: SyscallContext) -> KernelResult<u64> {
        let old_path_buf =
            self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let new_path_buf =
            self.resolve_user_path(context.caller, context.arg(2), context.arg(3))?;
        let flags = context.arg(4);
        if flags & !AT_SYMLINK_FOLLOW != 0 {
            return Err(KernelError::InvalidArgument);
        }
        self.root_fs
            .link(
                old_path_buf.as_path()?,
                new_path_buf.as_path()?,
                self.fs_credentials_for(context.caller)?,
            )
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn resolve_user_path(
        &self,
        pid: ProcessId,
        dirfd: u64,
        path_ptr: u64,
    ) -> KernelResult<KernelPathBuf> {
        let raw = self.user_path_str(path_ptr)?;
        self.resolve_path_str(pid, dirfd, raw)
    }

    fn resolve_path_str(
        &self,
        pid: ProcessId,
        dirfd: u64,
        raw: &str,
    ) -> KernelResult<KernelPathBuf> {
        Path::new_unchecked_rooted(raw).map_err(map_path_error)?;
        let files = self.process_files(pid)?;
        let root = files.root();
        let root_str = root.as_str();
        let root_len = root.len();
        let mut resolved = if raw.starts_with('/') {
            KernelPathBuf::from_str(root_str)?
        } else if dirfd as i32 == AT_FDCWD {
            KernelPathBuf::from_str(files.cwd().as_str())?
        } else {
            let description = files
                .get(dirfd as usize)
                .map_err(map_process_file_table_error)?
                .description();
            let file = self
                .open_files
                .get(description)
                .map_err(map_file_table_error)?;
            let metadata = self.root_fs.fstat(&file).map_err(KernelError::Filesystem)?;
            if metadata.kind != crate::kernel::fs::inode::InodeKind::Directory {
                return Err(KernelError::Filesystem(VfsError::NotDirectory));
            }
            KernelPathBuf::from_str(file.path())?
        };
        for component in raw.split('/') {
            let before = resolved.len;
            resolved.push_component(component)?;
            if component == ".." && resolved.len < root_len {
                resolved.truncate_to_root(root_len);
            } else if before < root_len {
                resolved.truncate_to_root(root_len);
            }
        }
        resolved.as_path()?;
        Ok(resolved)
    }

    fn user_path(&self, path_ptr: u64) -> KernelResult<Path<'_>> {
        let raw = self.user_path_str(path_ptr)?;
        Path::new(raw).map_err(map_path_error)
    }

    fn user_path_str(&self, path_ptr: u64) -> KernelResult<&'_ str> {
        let bytes = user_cstr(path_ptr)?;
        core::str::from_utf8(bytes).map_err(|_| KernelError::InvalidArgument)
    }

    fn replace_process_image(
        &mut self,
        pid: ProcessId,
        current_thread: Option<ThreadId>,
        entry_point: u64,
        stack_pointer: u64,
        address_space_root: u64,
    ) -> KernelResult<()> {
        let index = self.locate_process(pid)?;
        if let Some(pcb) = self.process_table[index].as_mut() {
            pcb.set_exec_image(entry_point, address_space_root);
            pcb.thread_count = 0;
        }

        let _ = self.mtss_scheduler.terminate_task(Self::mtss_task_id(pid));
        let mut kept_thread = current_thread;
        if kept_thread.is_none() {
            let mut idx = 0usize;
            while idx < Self::THREAD_CAPACITY {
                if let Some(thread) = self.thread_table[idx] {
                    if thread.process == pid {
                        kept_thread = Some(thread.id);
                        break;
                    }
                }
                idx += 1;
            }
        }

        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(thread) = self.thread_table[idx] {
                if thread.process == pid && Some(thread.id) != kept_thread {
                    self.thread_table[idx] = None;
                }
            }
            idx += 1;
        }

        let thread_id = if let Some(thread_id) = kept_thread {
            let thread_index = self.locate_thread(thread_id)?;
            let priority = self.process_table[index]
                .as_ref()
                .ok_or(KernelError::UnknownProcess)?
                .priority;
            if let Some(tcb) = self.thread_table[thread_index].as_mut() {
                tcb.replace_exec_image(entry_point, stack_pointer);
                tcb.priority = priority;
            }
            thread_id
        } else {
            self.create_thread(
                pid,
                entry_point,
                self.process_table[index]
                    .as_ref()
                    .ok_or(KernelError::UnknownProcess)?
                    .priority,
            )?
        };

        if let Some(pcb) = self.process_table[index].as_mut() {
            pcb.thread_count = 1;
            pcb.state = ProcessState::Ready;
        }
        let pcb = self.process_table[index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?;
        let priority = pcb.priority;
        let parent = pcb.parent;
        self.mtss_create_task(pid, parent, address_space_root, priority)
            .and_then(|_| self.mtss_create_thread(pid, thread_id, priority))
            .and_then(|_| self.mtss_enqueue_thread(thread_id))
    }

    fn wait_for_child(
        &mut self,
        parent: ProcessId,
        selector: i64,
        status_out: *mut i32,
        options: u64,
    ) -> KernelResult<u64> {
        const WNOHANG: u64 = 1;
        let mut saw_child = false;
        let parent_pgid = self.process_table[self.locate_process(parent)?]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .process_group;
        let mut idx = 0usize;
        while idx < MAX_PROC {
            if let Some(pcb) = self.process_table[idx] {
                if pcb.parent == Some(parent)
                    && wait_selector_matches(selector, pcb.pid, pcb.process_group, parent_pgid)
                {
                    saw_child = true;
                    if pcb.state == ProcessState::Zombie {
                        let status = pcb.exit_status.unwrap_or(ExitStatus::exited(0));
                        if !status_out.is_null() {
                            unsafe { status_out.write(status.raw()) };
                        }
                        let pid = pcb.pid;
                        self.reap_process_at(idx);
                        return Ok(pid.raw());
                    }
                }
            }
            idx += 1;
        }
        if saw_child && (options & WNOHANG) != 0 {
            Ok(0)
        } else if saw_child {
            Err(KernelError::MessageQueueEmpty)
        } else {
            Err(KernelError::UnknownProcess)
        }
    }

    fn reap_process_at(&mut self, index: usize) {
        if let Some(pcb) = self.process_table[index] {
            self.security.revoke_task(pcb.pid);
            self.process_table[index] = None;
            self.timers.release_process(pcb.pid);
        }
    }

    fn queue_signal_to_parent(&mut self, child: ProcessId, signal: u8) -> KernelResult<()> {
        let child_index = self.locate_process(child)?;
        if let Some(parent) = self.process_table[child_index]
            .as_ref()
            .and_then(|pcb| pcb.parent)
        {
            self.queue_signal(parent, signal)?;
        }
        Ok(())
    }

    fn queue_signal(&mut self, pid: ProcessId, signal: u8) -> KernelResult<()> {
        let index = self.locate_process(pid)?;
        self.process_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?
            .queue_signal(signal)
            .map_err(|_| KernelError::InvalidArgument)
    }

    fn send_signal(&mut self, selector: i64, signal: u8) -> KernelResult<u64> {
        if signal == 0 {
            self.ensure_process_exists(ProcessId::new(selector as u64))?;
            return Ok(0);
        }
        let mut delivered = 0u64;
        if selector > 0 {
            self.queue_signal(ProcessId::new(selector as u64), signal)?;
            return Ok(0);
        }
        let target_group = if selector == 0 {
            None
        } else {
            Some(ProcessGroupId::new((-selector) as u64))
        };
        let mut idx = 0usize;
        while idx < MAX_PROC {
            if let Some(pcb) = self.process_table[idx] {
                let matches = target_group
                    .map(|pgid| pcb.process_group == pgid)
                    .unwrap_or(true);
                if matches && pcb.state != ProcessState::Zombie {
                    if self.queue_signal(pcb.pid, signal).is_ok() {
                        delivered += 1;
                    }
                }
            }
            idx += 1;
        }
        if delivered == 0 {
            Err(KernelError::UnknownProcess)
        } else {
            Ok(0)
        }
    }

    fn deliver_signal_checkpoint(&mut self, pid: ProcessId, thread: ThreadId) -> KernelResult<()> {
        let thread_index = self.locate_thread(thread)?;
        let mask = self.thread_table[thread_index]
            .as_ref()
            .ok_or(KernelError::UnknownThread)?
            .signal_mask;
        let process_index = self.locate_process(pid)?;
        let Some(signal) = self.process_table[process_index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?
            .take_deliverable_signal(mask)
        else {
            return Ok(());
        };
        let action = self.process_table[process_index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .signal_actions[signal as usize];
        if action.handler == 0 && matches!(signal, SIGKILL | SIGTERM) {
            self.exit_process(pid, ExitStatus::signaled(signal));
            return Ok(());
        }
        if let Some(tcb) = self.thread_table[thread_index].as_mut() {
            tcb.deliver_signal(signal, action.handler);
        }
        Ok(())
    }

    fn process_credentials(
        &self,
        pid: ProcessId,
    ) -> KernelResult<&crate::kernel::process::ProcessCredentials> {
        let index = self.locate_process(pid)?;
        Ok(&self.process_table[index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .credentials)
    }

    fn process_credentials_mut(
        &mut self,
        pid: ProcessId,
    ) -> KernelResult<&mut crate::kernel::process::ProcessCredentials> {
        let index = self.locate_process(pid)?;
        Ok(&mut self.process_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?
            .credentials)
    }

    fn fs_credentials_for(&self, pid: ProcessId) -> KernelResult<FsCredentials> {
        let credentials = self.process_credentials(pid)?;
        Ok(FsCredentials::user_with_groups(
            credentials.euid,
            credentials.egid,
            credentials.supplementary_groups(),
            credentials.supplementary_group_count(),
        ))
    }

    fn process_files(
        &self,
        pid: ProcessId,
    ) -> KernelResult<&crate::kernel::process::ProcessFileTable<MAX_OPEN_FILES>> {
        let index = self.locate_process(pid)?;
        Ok(&self.process_table[index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .files)
    }

    fn process_files_mut(
        &mut self,
        pid: ProcessId,
    ) -> KernelResult<&mut crate::kernel::process::ProcessFileTable<MAX_OPEN_FILES>> {
        let index = self.locate_process(pid)?;
        Ok(&mut self.process_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?
            .files)
    }

    fn duplicate_fd(
        &mut self,
        pid: ProcessId,
        old_fd: usize,
        min_fd: usize,
        flags: DescriptorFlags,
    ) -> KernelResult<u64> {
        let description = self.fd_description(pid, old_fd)?;
        self.open_files
            .increment_ref_count(description)
            .map_err(map_file_table_error)?;
        match self
            .process_files_mut(pid)?
            .open_at_or_above(description, flags, min_fd)
        {
            Ok(fd) => Ok(fd as u64),
            Err(error) => {
                let _ = self.close_open_description(description);
                Err(map_process_file_table_error(error))
            }
        }
    }

    fn duplicate_fd_to(
        &mut self,
        pid: ProcessId,
        old_fd: usize,
        new_fd: usize,
        flags: DescriptorFlags,
        reject_same: bool,
    ) -> KernelResult<u64> {
        let description = self.fd_description(pid, old_fd)?;
        if old_fd == new_fd {
            if reject_same {
                return Err(KernelError::InvalidArgument);
            }
            return Ok(new_fd as u64);
        }
        self.open_files
            .increment_ref_count(description)
            .map_err(map_file_table_error)?;
        let replaced = match self
            .process_files_mut(pid)?
            .duplicate_to(new_fd, description, flags)
        {
            Ok(previous) => previous,
            Err(error) => {
                let _ = self.close_open_description(description);
                return Err(map_process_file_table_error(error));
            }
        };
        if let Some(previous) = replaced {
            self.close_open_description(previous.description())?;
        }
        Ok(new_fd as u64)
    }

    fn fd_description(&self, pid: ProcessId, fd: usize) -> KernelResult<FileDescriptionId> {
        self.process_files(pid)?
            .get(fd)
            .map(|descriptor| descriptor.description())
            .map_err(map_process_file_table_error)
    }

    fn inherit_process_file_table(
        &mut self,
        parent: ProcessId,
    ) -> KernelResult<crate::kernel::process::ProcessFileTable<MAX_OPEN_FILES>> {
        let inherited = *self.process_files(parent)?;
        let mut retained = 0usize;
        let descriptors = inherited.descriptors();
        while retained < MAX_OPEN_FILES {
            if let Some(descriptor) = descriptors[retained] {
                if let Err(error) = self
                    .open_files
                    .increment_ref_count(descriptor.description())
                {
                    self.release_inherited_prefix(&inherited, retained);
                    return Err(map_file_table_error(error));
                }
            }
            retained += 1;
        }
        Ok(inherited)
    }

    fn release_inherited_prefix(
        &mut self,
        table: &crate::kernel::process::ProcessFileTable<MAX_OPEN_FILES>,
        count: usize,
    ) {
        let mut idx = 0usize;
        while idx < count {
            if let Some(descriptor) = table.descriptors()[idx] {
                let _ = self.close_open_description(descriptor.description());
            }
            idx += 1;
        }
    }

    fn release_process_file_table(
        &mut self,
        table: &mut crate::kernel::process::ProcessFileTable<MAX_OPEN_FILES>,
    ) {
        let closed = table.clear();
        self.release_description_ids(&closed);
    }

    fn release_description_ids(
        &mut self,
        descriptions: &[Option<FileDescriptionId>; MAX_OPEN_FILES],
    ) {
        for description in descriptions.iter().flatten() {
            let _ = self.close_open_description(*description);
        }
    }

    fn close_open_description(&mut self, description: FileDescriptionId) -> KernelResult<()> {
        if let Some(object) = self
            .open_files
            .close(description)
            .map_err(map_file_table_error)?
        {
            match object {
                DescriptorObject::Regular(file) => {
                    self.root_fs.close(file).map_err(KernelError::Filesystem)?;
                }
                DescriptorObject::Pipe(endpoint) => self.release_pipe_endpoint(endpoint),
                DescriptorObject::EventFd(id) => self.release_eventfd(id),
                DescriptorObject::Device(_) | DescriptorObject::Socket(_) => {}
            }
        }
        Ok(())
    }

    fn fd_object(&self, pid: ProcessId, fd: usize) -> KernelResult<DescriptorObject> {
        let description = self.fd_description(pid, fd)?;
        self.open_files
            .get_object(description)
            .map_err(map_file_table_error)
    }

    fn socket_handle_for_fd(&self, pid: ProcessId, fd: usize) -> KernelResult<SocketHandle> {
        match self.fd_object(pid, fd)? {
            DescriptorObject::Socket(handle) => Ok(handle),
            _ => Err(KernelError::Filesystem(VfsError::InvalidHandle)),
        }
    }

    fn allocate_socket_handle(&mut self) -> KernelResult<SocketHandle> {
        let raw = self.next_socket_handle;
        if raw == 0 {
            return Err(KernelError::FileTableFull);
        }
        self.next_socket_handle = self
            .next_socket_handle
            .checked_add(1)
            .ok_or(KernelError::FileTableFull)?;
        Ok(SocketHandle::new(raw))
    }

    fn network_request_header(
        &mut self,
        opcode: NetworkOpcode,
        client: ProcessId,
        flags: u32,
    ) -> NetworkRequestHeader {
        NetworkRequestHeader::new(opcode, client, self.next_message_sequence(), flags)
    }

    fn send_network_request<T: NetworkIpcRequest>(
        &mut self,
        sender: ProcessId,
        request: &T,
    ) -> KernelResult<()> {
        let receiver = self
            .service_registry
            .owner(RegistryServiceId::Networkd)
            .ok_or(KernelError::UnknownProcess)?;
        self.ensure_process_exists(receiver)?;
        self.security
            .authorize_ipc(
                sender,
                receiver,
                RegistryServiceId::Networkd.security_class(),
            )
            .map_err(KernelError::SecurityViolation)?;
        let payload = MessagePayload::from_slice(
            RegistryServiceId::Networkd.security_class(),
            request.as_bytes(),
        );
        self.send_message(sender, receiver, payload)
    }

    fn allocate_pipe(&mut self) -> KernelResult<PipeId> {
        let mut idx = 0usize;
        while idx < MAX_KERNEL_PIPES {
            if self.pipes[idx].is_none() {
                self.pipes[idx] = Some(PipeObject::new());
                return Ok(PipeId::new(idx));
            }
            idx += 1;
        }
        Err(KernelError::FileTableFull)
    }

    fn release_pipe_endpoint(&mut self, endpoint: PipeEndpoint) {
        if let Some(pipe) = self
            .pipes
            .get_mut(endpoint.id().raw())
            .and_then(Option::as_mut)
        {
            match endpoint.direction() {
                PipeDirection::Read => pipe.readers = pipe.readers.saturating_sub(1),
                PipeDirection::Write => pipe.writers = pipe.writers.saturating_sub(1),
            }
            if pipe.readers == 0 && pipe.writers == 0 {
                self.pipes[endpoint.id().raw()] = None;
            }
        }
    }

    fn allocate_eventfd(&mut self, counter: u64, semaphore: bool) -> KernelResult<EventFdId> {
        let mut idx = 0usize;
        while idx < MAX_KERNEL_EVENTFDS {
            if self.eventfds[idx].is_none() {
                self.eventfds[idx] = Some(EventFdObject::new(counter, semaphore));
                return Ok(EventFdId::new(idx));
            }
            idx += 1;
        }
        Err(KernelError::FileTableFull)
    }

    fn release_eventfd(&mut self, id: EventFdId) {
        if id.raw() < MAX_KERNEL_EVENTFDS {
            self.eventfds[id.raw()] = None;
        }
    }

    fn descriptor_readiness(&self, object: DescriptorObject, events: i16) -> i16 {
        let mut revents = 0i16;
        match object {
            DescriptorObject::Regular(_) => {
                if events & (POLLIN | POLLPRI) != 0 {
                    revents |= events & (POLLIN | POLLPRI);
                }
                if events & POLLOUT != 0 {
                    revents |= POLLOUT;
                }
            }
            DescriptorObject::Pipe(endpoint) => {
                match self.pipes.get(endpoint.id().raw()).and_then(|entry| *entry) {
                    Some(pipe) => match endpoint.direction() {
                        PipeDirection::Read => {
                            if events & (POLLIN | POLLPRI) != 0 && pipe.is_readable() {
                                revents |= events & (POLLIN | POLLPRI);
                            }
                            if pipe.writers == 0 {
                                revents |= POLLHUP;
                            }
                        }
                        PipeDirection::Write => {
                            if events & POLLOUT != 0 && pipe.is_writable() {
                                revents |= POLLOUT;
                            }
                            if pipe.readers == 0 {
                                revents |= POLLERR;
                            }
                        }
                    },
                    None => revents |= POLLNVAL,
                }
            }
            DescriptorObject::EventFd(id) => {
                match self.eventfds.get(id.raw()).and_then(|entry| *entry) {
                    Some(eventfd) => {
                        if events & (POLLIN | POLLPRI) != 0 && eventfd.is_readable() {
                            revents |= events & (POLLIN | POLLPRI);
                        }
                        if events & POLLOUT != 0 && eventfd.is_writable() {
                            revents |= POLLOUT;
                        }
                    }
                    None => revents |= POLLNVAL,
                }
            }
            DescriptorObject::Device(_) | DescriptorObject::Socket(_) => {
                if events & (POLLIN | POLLPRI) != 0 {
                    revents |= events & (POLLIN | POLLPRI);
                }
                if events & POLLOUT != 0 {
                    revents |= POLLOUT;
                }
            }
        }
        revents
    }

    fn poll_descriptors(
        &mut self,
        pid: ProcessId,
        thread: Option<ThreadId>,
        fds: &mut [MiragePollFd],
        timeout_ms: i32,
    ) -> KernelResult<u64> {
        let mut ready = 0u64;
        let mut idx = 0usize;
        while idx < fds.len() {
            fds[idx].revents = 0;
            if fds[idx].fd >= 0 {
                match self.fd_object(pid, fds[idx].fd as usize) {
                    Ok(object) => {
                        fds[idx].revents = self.descriptor_readiness(object, fds[idx].events)
                    }
                    Err(_) => fds[idx].revents = POLLNVAL,
                }
                if fds[idx].revents != 0 {
                    ready += 1;
                }
            }
            idx += 1;
        }
        if ready == 0 && timeout_ms != 0 {
            if timeout_ms > 0 {
                let wake_deadline = KERNEL_TIME
                    .now()
                    .as_nanos()
                    .saturating_add((timeout_ms as u128).saturating_mul(1_000_000));
                self.timers
                    .add_sleep(pid, thread, wake_deadline)
                    .map_err(map_timer_error)?;
            }
            let process_index = self.locate_process(pid)?;
            self.block_process_at_index(pid, process_index);
        }
        Ok(ready)
    }

    pub fn tick(&mut self) {
        self.kernel_on_timer_tick();
        device::system_timer().tick();
        let timestamp = KERNEL_TIME.tick();
        let now_ns = timestamp.as_nanos();
        self.wake_expired_timeouts(now_ns);
        self.wake_expired_futexes(now_ns);
        let mut core_index = 0usize;
        while core_index < cpu::MAX_CORES {
            if self.core_states[core_index].online {
                self.run_core(core_index);
            }
            core_index += 1;
        }
    }

    fn wake_expired_timeouts(&mut self, now_ns: u128) {
        while let Some(expired) = self.timers.expire_sleep(now_ns) {
            let _ = self.wake_process_for_timeout(expired.process);
        }

        while let Some(expired) = self.timers.expire_timer(now_ns) {
            let _ = self.wake_process_for_timeout(expired.owner);
        }
    }

    fn wake_expired_futexes(&mut self, now_ns: u128) {
        let mut expired_threads = [None; MAX_THREADS];
        let count = self.futexes.expire(now_ns, &mut expired_threads);
        let _ = self.wake_futex_threads(
            &expired_threads,
            count,
            encode_syscall_error(KernelError::TimedOut),
        );
    }

    fn wake_process_for_timeout(&mut self, pid: ProcessId) -> KernelResult<()> {
        let index = self.locate_process(pid)?;
        let mut wake_threads = false;
        if let Some(pcb) = self.process_table[index].as_mut() {
            if pcb.state == ProcessState::Blocked {
                pcb.state = ProcessState::Ready;
                wake_threads = true;
            }
        }
        if wake_threads {
            self.make_threads_ready(pid)?;
        }
        Ok(())
    }

    fn run_core(&mut self, core_index: usize) {
        if let Some(scheduled) = self.kernel_schedule_next() {
            let thread_index = match self.locate_thread(scheduled.thread) {
                Ok(idx) => idx,
                Err(_) => {
                    self.core_states[core_index].idle_cycle();
                    return;
                }
            };

            let process_index = match self.locate_process(scheduled.process) {
                Ok(idx) => idx,
                Err(_) => {
                    self.thread_table[thread_index] = None;
                    self.core_states[core_index].idle_cycle();
                    return;
                }
            };

            if let Err(reason) = self.security.enforce_isolation(scheduled.process) {
                self.handle_isolation_fault(scheduled.process, reason);
                return;
            }

            let _ = self.deliver_signal_checkpoint(scheduled.process, scheduled.thread);
            if self.locate_thread(scheduled.thread).is_err() {
                self.core_states[core_index].idle_cycle();
                return;
            }

            let address_space_root = self.process_table[process_index]
                .as_ref()
                .map(|pcb| pcb.address_space_root)
                .unwrap_or(0);
            if address_space_root == 0 {
                self.handle_isolation_fault(scheduled.process, IsolationError::PolicyViolation);
                return;
            }

            let kernel_stack_top = x86_64::kernel_stack_top(core_index);
            self.core_states[core_index].set_kernel_stack_top(kernel_stack_top);
            self.core_states[core_index].start_thread(scheduled.thread);

            let mut terminated = false;
            let mut run_outcome = ThreadRunOutcome::TimeSliceComplete;
            if let Some(entry) = self.thread_table.get_mut(thread_index) {
                if let Some(thread) = entry.as_mut() {
                    if thread.state == ThreadState::Terminated {
                        *entry = None;
                        terminated = true;
                    } else {
                        run_outcome = x86_64::run_thread_slice(ThreadSliceRunContext {
                            core_index,
                            thread: scheduled.thread,
                            process: scheduled.process,
                            address_space_root,
                            kernel_stack_top,
                            context: &mut thread.context,
                        });
                        if run_outcome != ThreadRunOutcome::UserEntryInvalid {
                            thread.mark_running();
                            thread.accumulate_cpu_time(1);
                        }
                    }
                }
            }

            if terminated {
                self.update_process_thread_count(scheduled.process, false);
                self.core_states[core_index].finish_cycle();
                return;
            }

            if let Some(pcb) = self.process_table[process_index].as_mut() {
                pcb.state = ProcessState::Running;
                pcb.cpu_time = pcb.cpu_time.saturating_add(1);
            }

            match run_outcome {
                ThreadRunOutcome::Syscall(trap) => {
                    let context =
                        SyscallContext::new(scheduled.process, Some(trap.thread), trap.args);
                    let result = self
                        .handle_syscall(trap.number, context)
                        .unwrap_or_else(encode_syscall_error);
                    self.write_thread_syscall_result(trap.thread, result);
                    let _ = self.deliver_signal_checkpoint(scheduled.process, trap.thread);
                }
                ThreadRunOutcome::TimerPreempted | ThreadRunOutcome::TimeSliceComplete => {}
                ThreadRunOutcome::UserEntryInvalid => {
                    self.handle_isolation_fault(scheduled.process, IsolationError::PolicyViolation);
                }
            }

            let mut requeue_thread = false;
            if let Some(entry) = self.thread_table.get_mut(thread_index) {
                if let Some(thread) = entry.as_mut() {
                    if thread.state == ThreadState::Running {
                        thread.mark_ready();
                    }
                    requeue_thread = thread.state == ThreadState::Ready;
                }
            }

            let process_has_runnable_threads = self.has_runnable_thread(scheduled.process);
            if let Some(pcb) = self.process_table[process_index].as_mut() {
                if pcb.state == ProcessState::Running {
                    pcb.state = if process_has_runnable_threads {
                        ProcessState::Ready
                    } else {
                        ProcessState::Blocked
                    };
                }
            }

            self.core_states[core_index].finish_cycle();

            if requeue_thread {
                match self.kernel_yield_current(scheduled) {
                    Ok(Some(next)) => {
                        // MTSS has already selected the next runnable thread. The
                        // single-slice core loop defers dispatching that exact MTSS
                        // decision until the next scheduler tick, where
                        // `kernel_schedule_next` will expose only MTSS-selected
                        // threads to the architecture backend.
                        self.pending_mtss_decision = Some(next);
                    }
                    Ok(None) => {}
                    Err(_) => {
                        self.core_states[core_index].idle_cycle();
                    }
                }
            }
        } else {
            self.core_states[core_index].idle_cycle();
        }
    }

    fn futex_owner_for_process(&self, pid: ProcessId) -> u64 {
        if let Ok(process_index) = self.locate_process(pid) {
            if let Some(pcb) = self.process_table[process_index].as_ref() {
                if pcb.address_space_root != 0 {
                    return pcb.address_space_root;
                }
            }
        }
        pid.raw()
    }

    fn futex_key(&self, pid: ProcessId, user_address: u64) -> KernelResult<FutexKey> {
        let _ = user_out_ptr::<i32>(user_address)?;
        let process_index = self.locate_process(pid)?;
        let owner = self.process_table[process_index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .address_space_root;
        let owner = if owner == 0 { pid.raw() } else { owner };
        Ok(FutexKey::new(owner, user_address))
    }

    fn set_thread_fs_base(&mut self, thread: ThreadId, base: u64) -> KernelResult<u64> {
        let index = self.locate_thread(thread)?;
        let tcb = self.thread_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownThread)?;
        tcb.set_fs_base(base);
        Ok(0)
    }

    fn set_thread_gs_base(&mut self, thread: ThreadId, base: u64) -> KernelResult<u64> {
        let index = self.locate_thread(thread)?;
        let tcb = self.thread_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownThread)?;
        tcb.set_gs_base(base);
        Ok(0)
    }

    fn thread_fs_base(&self, thread: ThreadId) -> KernelResult<u64> {
        let index = self.locate_thread(thread)?;
        self.thread_table[index]
            .map(|tcb| tcb.fs_base)
            .ok_or(KernelError::UnknownThread)
    }

    fn thread_gs_base(&self, thread: ThreadId) -> KernelResult<u64> {
        let index = self.locate_thread(thread)?;
        self.thread_table[index]
            .map(|tcb| tcb.gs_base)
            .ok_or(KernelError::UnknownThread)
    }

    fn block_thread(&mut self, thread: ThreadId) -> KernelResult<()> {
        let index = self.locate_thread(thread)?;
        let process = self.thread_table[index]
            .as_ref()
            .ok_or(KernelError::UnknownThread)?
            .process;
        if let Some(tcb) = self.thread_table[index].as_mut() {
            tcb.block();
        }
        self.mtss_scheduler
            .block_thread(Self::mtss_thread_id(thread))
            .map_err(map_mtss_error)?;
        if !self.has_runnable_thread(process) {
            if let Ok(process_index) = self.locate_process(process) {
                if let Some(pcb) = self.process_table[process_index].as_mut() {
                    if pcb.state != ProcessState::Zombie {
                        pcb.state = ProcessState::Blocked;
                    }
                }
            }
        }
        Ok(())
    }

    fn wake_thread(&mut self, thread: ThreadId) -> KernelResult<()> {
        let index = self.locate_thread(thread)?;
        let mut process = None;
        if let Some(tcb) = self.thread_table[index].as_mut() {
            if tcb.state == ThreadState::Blocked {
                tcb.mark_ready();
                process = Some(tcb.process);
            }
        }
        if let Some(process) = process {
            self.mtss_scheduler
                .wake_thread(Self::mtss_thread_id(thread))
                .map_err(map_mtss_error)?;
            if let Ok(process_index) = self.locate_process(process) {
                if let Some(pcb) = self.process_table[process_index].as_mut() {
                    if pcb.state == ProcessState::Blocked {
                        pcb.state = ProcessState::Ready;
                    }
                }
            }
        }
        Ok(())
    }

    fn has_runnable_thread(&self, pid: ProcessId) -> bool {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(thread) = self.thread_table[idx] {
                if thread.process == pid
                    && (thread.state == ThreadState::Ready || thread.state == ThreadState::Running)
                {
                    return true;
                }
            }
            idx += 1;
        }
        false
    }

    fn wake_futex_threads(
        &mut self,
        threads: &[Option<ThreadId>],
        count: usize,
        result: u64,
    ) -> KernelResult<()> {
        let mut idx = 0usize;
        while idx < count && idx < threads.len() {
            if let Some(thread) = threads[idx] {
                self.write_thread_syscall_result(thread, result);
                self.wake_thread(thread)?;
            }
            idx += 1;
        }
        Ok(())
    }

    fn block_process_at_index(&mut self, pid: ProcessId, index: usize) {
        if let Some(pcb) = self.process_table[index].as_mut() {
            pcb.state = ProcessState::Blocked;
        }
        self.block_threads_for_process(pid);
    }

    fn block_threads_for_process(&mut self, pid: ProcessId) {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(entry) = self.thread_table.get_mut(idx) {
                if let Some(thread) = entry.as_mut() {
                    if thread.process == pid {
                        thread.block();
                    }
                }
            }
            idx += 1;
        }
    }

    fn make_threads_ready(&mut self, pid: ProcessId) -> KernelResult<()> {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(entry) = self.thread_table.get_mut(idx) {
                if let Some(thread) = entry.as_mut() {
                    if thread.process == pid && thread.state == ThreadState::Blocked {
                        thread.mark_ready();
                        if self
                            .mtss_scheduler
                            .wake_thread(Self::mtss_thread_id(thread.id))
                            .is_err()
                        {
                            thread.block();
                            self.rollback_ready_threads(pid, idx);
                            return Err(KernelError::SchedulerFull);
                        }
                    }
                }
            }
            idx += 1;
        }
        Ok(())
    }

    fn rollback_ready_threads(&mut self, pid: ProcessId, before_index: usize) {
        let mut idx = 0usize;
        while idx < before_index {
            if let Some(entry) = self.thread_table.get_mut(idx) {
                if let Some(thread) = entry.as_mut() {
                    if thread.process == pid && thread.state == ThreadState::Ready {
                        thread.block();
                        let _ = self
                            .mtss_scheduler
                            .block_thread(Self::mtss_thread_id(thread.id));
                    }
                }
            }
            idx += 1;
        }
    }

    fn remove_threads_for_process(&mut self, pid: ProcessId) {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(thread) = self.thread_table[idx] {
                if thread.process == pid {
                    let _ = self
                        .mtss_scheduler
                        .exit_thread(Self::mtss_thread_id(thread.id));
                    self.futexes.remove_thread(thread.id);
                    self.remove_thread_from_cores(thread.id);
                    self.thread_table[idx] = None;
                }
            }
            idx += 1;
        }
    }

    fn remove_thread_from_cores(&mut self, thread: ThreadId) {
        let mut idx = 0usize;
        while idx < cpu::MAX_CORES {
            self.core_states[idx].evict(thread);
            idx += 1;
        }
    }

    fn create_thread(
        &mut self,
        pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
    ) -> KernelResult<ThreadId> {
        let slot = self
            .find_free_thread_slot()
            .ok_or(KernelError::ThreadTableFull)?;
        let id = self.allocate_thread_id();
        let stack_pointer = self.allocate_stack_pointer(slot, id);
        let tcb = ThreadControlBlock::new(id, pid, entry_point, priority, stack_pointer);
        self.thread_table[slot] = Some(tcb);
        self.update_process_thread_count(pid, true);
        Ok(id)
    }

    fn rollback_thread_creation(&mut self, thread: ThreadId) {
        if let Ok(index) = self.locate_thread(thread) {
            if let Some(tcb) = self.thread_table[index] {
                self.futexes.remove_thread(thread);
                self.thread_table[index] = None;
                self.update_process_thread_count(tcb.process, false);
            }
        }
    }

    fn write_thread_syscall_result(&mut self, thread: ThreadId, result: u64) {
        if let Ok(index) = self.locate_thread(thread) {
            if let Some(tcb) = self.thread_table[index].as_mut() {
                tcb.write_syscall_result(result);
            }
        }
    }

    fn allocate_stack_pointer(&self, slot: usize, thread: ThreadId) -> u64 {
        const USER_STACK_BASE: u64 = 0x0000_7000_0000_0000;
        const USER_STACK_SIZE: u64 = 0x20_000;
        let stack_slot = (slot as u64).saturating_add(thread.raw());
        USER_STACK_BASE.saturating_add(stack_slot.saturating_mul(USER_STACK_SIZE))
    }

    fn update_process_thread_count(&mut self, pid: ProcessId, increment: bool) {
        if let Ok(index) = self.locate_process(pid) {
            if let Some(pcb) = self.process_table[index].as_mut() {
                if increment {
                    pcb.increment_thread_count();
                } else {
                    pcb.decrement_thread_count();
                }
            }
        }
    }

    fn ensure_process_exists(&self, pid: ProcessId) -> KernelResult<()> {
        self.locate_process(pid).map(|_| ())
    }

    fn handle_isolation_fault(&mut self, pid: ProcessId, _reason: IsolationError) {
        self.terminate_process(pid);
    }

    fn find_free_slot(&self) -> Option<usize> {
        let mut idx = 0;
        while idx < MAX_PROC {
            if self.process_table[idx].is_none() {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    fn find_free_thread_slot(&self) -> Option<usize> {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if self.thread_table[idx].is_none() {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    fn locate_process(&self, pid: ProcessId) -> KernelResult<usize> {
        let mut idx = 0;
        while idx < MAX_PROC {
            if let Some(pcb) = &self.process_table[idx] {
                if pcb.pid == pid {
                    return Ok(idx);
                }
            }
            idx += 1;
        }
        Err(KernelError::UnknownProcess)
    }

    fn locate_thread(&self, thread: ThreadId) -> KernelResult<usize> {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(tcb) = &self.thread_table[idx] {
                if tcb.id == thread {
                    return Ok(idx);
                }
            }
            idx += 1;
        }
        Err(KernelError::UnknownThread)
    }

    fn allocate_pid(&mut self) -> ProcessId {
        let pid = ProcessId::new(self.next_pid);
        self.next_pid += 1;
        pid
    }

    fn allocate_thread_id(&mut self) -> ThreadId {
        let id = ThreadId::new(self.next_thread);
        self.next_thread += 1;
        id
    }

    fn next_message_sequence(&mut self) -> u64 {
        let seq = self.message_sequence;
        self.message_sequence = self.message_sequence.wrapping_add(1);
        seq
    }

    pub fn enumerate_devices(&self, out: &mut [DeviceDescriptor]) -> usize {
        self.devices.enumerate(out)
    }

    pub fn device_info(&self, id: DeviceId) -> Option<DeviceDescriptor> {
        self.devices.descriptor(id)
    }

    pub fn device_read(
        &self,
        pid: ProcessId,
        id: DeviceId,
        buffer: &mut [u8],
    ) -> KernelResult<usize> {
        let descriptor = self
            .devices
            .descriptor(id)
            .ok_or(KernelError::DeviceNotFound)?;

        self.security
            .authorize_device_access(
                pid,
                CapabilityObject::PciDevice(descriptor.id.raw() as u64),
                CapabilityRight::Read,
                descriptor.security,
            )
            .map_err(KernelError::SecurityViolation)?;
        if !self.service_registry.claimed_by(pid, id) {
            return Err(KernelError::SecurityViolation(
                IsolationError::PolicyViolation,
            ));
        }

        self.devices
            .read(id, buffer)
            .map_err(KernelError::DeviceFault)
    }

    pub fn device_write(&self, pid: ProcessId, id: DeviceId, data: &[u8]) -> KernelResult<usize> {
        let descriptor = self
            .devices
            .descriptor(id)
            .ok_or(KernelError::DeviceNotFound)?;

        self.security
            .authorize_device_access(
                pid,
                CapabilityObject::PciDevice(descriptor.id.raw() as u64),
                CapabilityRight::Write,
                descriptor.security,
            )
            .map_err(KernelError::SecurityViolation)?;
        if !self.service_registry.claimed_by(pid, id) {
            return Err(KernelError::SecurityViolation(
                IsolationError::PolicyViolation,
            ));
        }

        self.devices
            .write(id, data)
            .map_err(KernelError::DeviceFault)
    }
}

fn exec_vector_metadata(ptr: u64, max_entries: usize) -> KernelResult<ExecVectorMetadata> {
    if ptr == 0 {
        return Ok(ExecVectorMetadata::empty());
    }

    let entries = user_slice_typed::<u64>(ptr, max_entries + 1)?;
    let mut count = 0usize;
    while count < entries.len() {
        if entries[count] == 0 {
            return Ok(ExecVectorMetadata::new(ptr, count, false));
        }
        count += 1;
    }

    Ok(ExecVectorMetadata::new(ptr, max_entries, true))
}

fn signed_exec_manifest_for_path(
    path: &str,
) -> (Option<ExecServiceDaemon>, Option<ExecSignatureMetadata>) {
    match path {
        "/bin/displayd" | "/sbin/displayd" | "/displayd" => (
            Some(ExecServiceDaemon::Display),
            Some(ExecSignatureMetadata::new(
                "mirage-service-root",
                0x444953504c415944,
            )),
        ),
        "/bin/networkd" | "/sbin/networkd" | "/networkd" => (
            Some(ExecServiceDaemon::Network),
            Some(ExecSignatureMetadata::new(
                "mirage-service-root",
                0x4e4554574f524b44,
            )),
        ),
        "/bin/inputd" | "/sbin/inputd" | "/inputd" => (
            Some(ExecServiceDaemon::Input),
            Some(ExecSignatureMetadata::new(
                "mirage-service-root",
                0x494e50555444414d,
            )),
        ),
        "/bin/l2-driverd" | "/sbin/l2-driverd" | "/l2-driverd" => (
            Some(ExecServiceDaemon::L2Driver),
            Some(ExecSignatureMetadata::new(
                "mirage-driver-root",
                0x4c32445249564552,
            )),
        ),
        _ => (None, None),
    }
}

fn map_file_table_error(error: FileTableError) -> KernelError {
    match error {
        FileTableError::Full => KernelError::FileTableFull,
        FileTableError::InvalidDescriptor => KernelError::Filesystem(VfsError::InvalidHandle),
    }
}

fn map_process_file_table_error(error: ProcessFileTableError) -> KernelError {
    match error {
        ProcessFileTableError::Full => KernelError::FileTableFull,
        ProcessFileTableError::InvalidDescriptor => {
            KernelError::Filesystem(VfsError::InvalidHandle)
        }
    }
}

fn map_path_error(error: PathError) -> KernelError {
    KernelError::Filesystem(VfsError::InvalidPath(error))
}

fn map_service_registry_error(error: ServiceRegistryError) -> KernelError {
    match error {
        ServiceRegistryError::Full => KernelError::ProcessTableFull,
        ServiceRegistryError::AlreadyRegistered
        | ServiceRegistryError::DeviceAlreadyClaimed
        | ServiceRegistryError::DeviceClassMismatch
        | ServiceRegistryError::NotOwner => KernelError::InvalidArgument,
        ServiceRegistryError::NotRegistered => KernelError::UnknownProcess,
        ServiceRegistryError::DeviceNotClaimed => KernelError::DeviceNotFound,
    }
}

fn decode_registry_service_id(raw: u64) -> KernelResult<RegistryServiceId> {
    RegistryServiceId::from_raw(raw).ok_or(KernelError::InvalidArgument)
}

fn validate_tls_base(base: u64) -> KernelResult<()> {
    if base < USER_CANONICAL_LIMIT {
        Ok(())
    } else {
        Err(KernelError::InvalidArgument)
    }
}

fn active_user_root() -> u64 {
    x86_64::paging::current_address_space_root()
}

fn validate_user_range(ptr: u64, len: usize) -> KernelResult<()> {
    validate_user_access(ptr, len, false)
}

fn validate_user_access(ptr: u64, len: usize, write: bool) -> KernelResult<()> {
    if len == 0 {
        return Ok(());
    }
    if ptr == 0 {
        return Err(KernelError::InvalidPointer);
    }
    ptr.checked_add(len as u64)
        .filter(|end| *end >= ptr && *end <= USER_CANONICAL_LIMIT)
        .ok_or(KernelError::InvalidPointer)?;
    if !x86_64::paging::installed() || active_user_root() == 0 {
        return Ok(());
    }
    if !memory::validate_user_range(active_user_root(), ptr, len, write) {
        return Err(KernelError::InvalidPointer);
    }
    Ok(())
}

fn user_translated_ptr(ptr: u64, len: usize, write: bool) -> KernelResult<NonNull<u8>> {
    validate_user_access(ptr, len, write)?;
    if len == 0 {
        return Ok(NonNull::dangling());
    }
    if !x86_64::paging::installed() || active_user_root() == 0 {
        return NonNull::new(ptr as *mut u8).ok_or(KernelError::InvalidPointer);
    }
    memory::active_translated_slice(active_user_root(), ptr, len, write)
        .ok_or(KernelError::InvalidPointer)
}

fn user_slice(ptr: u64, len: usize) -> KernelResult<&'static [u8]> {
    let translated = user_translated_ptr(ptr, len, false)?;
    if len == 0 {
        Ok(&[])
    } else {
        Ok(unsafe { core::slice::from_raw_parts(translated.as_ptr(), len) })
    }
}

fn user_slice_mut(ptr: u64, len: usize) -> KernelResult<&'static mut [u8]> {
    let translated = user_translated_ptr(ptr, len, true)?;
    if len == 0 {
        Ok(&mut [])
    } else {
        Ok(unsafe { core::slice::from_raw_parts_mut(translated.as_ptr(), len) })
    }
}

fn user_slice_typed<T>(ptr: u64, count: usize) -> KernelResult<&'static [T]> {
    let byte_len = count
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(KernelError::InvalidPointer)?;
    if count == 0 {
        return Ok(&[]);
    }
    if !(ptr as *const T).is_aligned() {
        return Err(KernelError::InvalidPointer);
    }
    let translated = user_translated_ptr(ptr, byte_len, false)?;
    if !(translated.as_ptr() as *const T).is_aligned() {
        return Err(KernelError::InvalidPointer);
    }
    Ok(unsafe { core::slice::from_raw_parts(translated.as_ptr() as *const T, count) })
}

fn user_slice_mut_typed<T>(ptr: u64, count: usize) -> KernelResult<&'static mut [T]> {
    let byte_len = count
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(KernelError::InvalidPointer)?;
    if count == 0 {
        return Ok(&mut []);
    }
    if !(ptr as *mut T).is_aligned() {
        return Err(KernelError::InvalidPointer);
    }
    let translated = user_translated_ptr(ptr, byte_len, true)?;
    if !(translated.as_ptr() as *mut T).is_aligned() {
        return Err(KernelError::InvalidPointer);
    }
    Ok(unsafe { core::slice::from_raw_parts_mut(translated.as_ptr() as *mut T, count) })
}

fn user_out_ptr<T>(ptr: u64) -> KernelResult<*mut T> {
    if !(ptr as *mut T).is_aligned() {
        return Err(KernelError::InvalidPointer);
    }
    let translated = user_translated_ptr(ptr, core::mem::size_of::<T>(), true)?;
    if !(translated.as_ptr() as *mut T).is_aligned() {
        return Err(KernelError::InvalidPointer);
    }
    Ok(translated.as_ptr() as *mut T)
}

fn read_user_value<T: Copy>(ptr: u64) -> KernelResult<T> {
    if !(ptr as *const T).is_aligned() {
        return Err(KernelError::InvalidPointer);
    }
    let mut value = core::mem::MaybeUninit::<T>::uninit();
    let out = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr() as *mut u8, core::mem::size_of::<T>())
    };
    if !x86_64::paging::installed() || active_user_root() == 0 {
        unsafe { core::ptr::copy_nonoverlapping(ptr as *const u8, out.as_mut_ptr(), out.len()) };
    } else if !memory::copy_from_user(active_user_root(), ptr, out) {
        return Err(KernelError::InvalidPointer);
    }
    Ok(unsafe { value.assume_init() })
}

fn write_user_value<T: Copy>(ptr: u64, value: T) -> KernelResult<()> {
    if !(ptr as *const T).is_aligned() {
        return Err(KernelError::InvalidPointer);
    }
    let input = unsafe {
        core::slice::from_raw_parts((&value as *const T) as *const u8, core::mem::size_of::<T>())
    };
    if !x86_64::paging::installed() || active_user_root() == 0 {
        unsafe { core::ptr::copy_nonoverlapping(input.as_ptr(), ptr as *mut u8, input.len()) };
    } else if !memory::copy_to_user(active_user_root(), ptr, input) {
        return Err(KernelError::InvalidPointer);
    }
    Ok(())
}

fn timespec_to_nanos(timespec: MirageTimespec) -> KernelResult<u128> {
    if timespec.tv_sec < 0 || timespec.tv_nsec < 0 || timespec.tv_nsec >= 1_000_000_000 {
        return Err(KernelError::InvalidArgument);
    }
    let seconds = timespec.tv_sec as u128;
    let nanos = timespec.tv_nsec as u128;
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_add(nanos))
        .ok_or(KernelError::InvalidArgument)
}

fn nanos_to_timespec(nanos: u128) -> MirageTimespec {
    MirageTimespec {
        tv_sec: (nanos / 1_000_000_000) as i64,
        tv_nsec: (nanos % 1_000_000_000) as i64,
    }
}

fn descriptor_flags_from_raw(flags: u64) -> DescriptorFlags {
    if flags & (O_CLOEXEC_RAW | EFD_CLOEXEC) != 0 {
        DescriptorFlags::CLOSE_ON_EXEC
    } else {
        DescriptorFlags::EMPTY
    }
}

fn timer_to_itimerspec(
    timer: crate::kernel::timer::ProcessTimer,
    now_ns: u128,
) -> MirageItimerspec {
    let remaining = if timer.armed {
        timer.wake_deadline_ns.saturating_sub(now_ns)
    } else {
        0
    };
    MirageItimerspec {
        it_interval: nanos_to_timespec(timer.interval_ns),
        it_value: nanos_to_timespec(remaining),
    }
}

fn map_timer_error(error: TimerError) -> KernelError {
    match error {
        TimerError::Full => KernelError::AllocationFailed,
        TimerError::InvalidTimer => KernelError::InvalidArgument,
    }
}

fn user_cstr(ptr: u64) -> KernelResult<&'static [u8]> {
    validate_user_range(ptr, 1)?;
    let mut len = 0usize;
    while len <= MAX_PATH_BYTES {
        let mut byte = [0u8; 1];
        if !x86_64::paging::installed() || active_user_root() == 0 {
            byte[0] = unsafe { ((ptr + len as u64) as *const u8).read() };
        } else if !memory::copy_from_user(active_user_root(), ptr + len as u64, &mut byte) {
            return Err(KernelError::InvalidPointer);
        }
        if byte[0] == 0 {
            return user_slice(ptr, len);
        }
        len += 1;
    }
    Err(KernelError::Filesystem(VfsError::InvalidPath(
        PathError::TooLong,
    )))
}

fn encode_syscall_error(error: KernelError) -> u64 {
    MIRAGE_SYSCALL_ERROR_BIT | syscall_error_code(error).raw()
}

fn syscall_error_code(error: KernelError) -> SyscallErrorCode {
    match error {
        KernelError::ProcessTableFull => SyscallErrorCode::ProcessTableFull,
        KernelError::SchedulerFull => SyscallErrorCode::SchedulerFull,
        KernelError::UnknownProcess => SyscallErrorCode::NoSuchProcess,
        KernelError::UnknownThread => SyscallErrorCode::NoSuchThread,
        KernelError::ThreadTableFull => SyscallErrorCode::ThreadTableFull,
        KernelError::MessageQueueFull => SyscallErrorCode::QueueFull,
        KernelError::MessageQueueEmpty => SyscallErrorCode::QueueEmpty,
        KernelError::SecurityViolation(reason) => isolation_syscall_error_code(reason),
        KernelError::IsolationFault(reason) => isolation_syscall_error_code(reason),
        KernelError::DeviceNotFound => SyscallErrorCode::NoSuchDevice,
        KernelError::DeviceFault(_) => SyscallErrorCode::DeviceFault,
        KernelError::InvalidSyscall => SyscallErrorCode::InvalidSyscall,
        KernelError::InvalidArgument => SyscallErrorCode::InvalidArgument,
        KernelError::InvalidPointer => SyscallErrorCode::BadAddress,
        KernelError::AllocationFailed => SyscallErrorCode::OutOfMemory,
        KernelError::FileTableFull => SyscallErrorCode::OutOfMemory,
        KernelError::Filesystem(error) => vfs_syscall_error_code(error),
        KernelError::TimedOut => SyscallErrorCode::TimedOut,
        KernelError::Loader(_) => SyscallErrorCode::InvalidArgument,
    }
}

fn vfs_syscall_error_code(error: VfsError) -> SyscallErrorCode {
    syscall_error_code_from_vfs(error)
}

fn isolation_syscall_error_code(reason: IsolationError) -> SyscallErrorCode {
    match reason {
        IsolationError::UnknownTask => SyscallErrorCode::NoSuchProcess,
        IsolationError::PolicyViolation | IsolationError::CapabilityMissing => {
            SyscallErrorCode::PermissionDenied
        }
        IsolationError::CapabilityTableFull => SyscallErrorCode::OutOfMemory,
    }
}

fn decode_priority(raw: u64) -> KernelResult<ProcessPriority> {
    match raw {
        0 => Ok(ProcessPriority::Critical),
        1 => Ok(ProcessPriority::High),
        2 => Ok(ProcessPriority::Normal),
        3 => Ok(ProcessPriority::Low),
        _ => Err(KernelError::InvalidArgument),
    }
}

fn decode_credentials(raw: u64) -> KernelResult<Credentials> {
    match raw {
        0 => Ok(Credentials::user()),
        1 => Ok(Credentials::system()),
        _ => Err(KernelError::InvalidArgument),
    }
}

fn decode_security_class(raw: u64) -> KernelResult<SecurityClass> {
    match raw {
        0 => Ok(SecurityClass::Public),
        1 => Ok(SecurityClass::Internal),
        2 => Ok(SecurityClass::Confidential),
        3 => Ok(SecurityClass::System),
        _ => Err(KernelError::InvalidArgument),
    }
}

#[cfg(all(test, not(feature = "qfs-std")))]
mod tests {
    use super::*;
    use crate::kernel::memory::{PROT_EXECUTE, PROT_READ, PROT_WRITE};
    use crate::libc;
    use crate::subkernel::{CapabilitySet, IsolationLevel, SecurityLabel};

    fn boot_kernel() -> Kernel<16, 4> {
        let mut kernel = Kernel::<16, 4>::new();
        kernel.bootstrap();
        kernel
    }

    fn process_state(kernel: &Kernel<16, 4>, pid: ProcessId) -> ProcessState {
        let index = kernel.locate_process(pid).unwrap();
        kernel.process_table[index].unwrap().state
    }

    fn first_thread(kernel: &Kernel<16, 4>, pid: ProcessId) -> ThreadId {
        let mut idx = 0usize;
        while idx < Kernel::<16, 4>::THREAD_CAPACITY {
            if let Some(thread) = kernel.thread_table[idx] {
                if thread.process == pid {
                    return thread.id;
                }
            }
            idx += 1;
        }
        panic!("process has no thread")
    }

    fn process_threads_blocked(kernel: &Kernel<16, 4>, pid: ProcessId) -> bool {
        let mut saw_thread = false;
        let mut idx = 0usize;
        while idx < Kernel::<16, 4>::THREAD_CAPACITY {
            if let Some(thread) = kernel.thread_table[idx] {
                if thread.process == pid {
                    saw_thread = true;
                    if thread.state != ThreadState::Blocked {
                        return false;
                    }
                }
            }
            idx += 1;
        }
        saw_thread
    }

    #[test]
    fn kernel_yield_current_returns_and_defers_mtss_selected_thread() {
        let mut kernel = boot_kernel();
        let first = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let second = kernel
            .spawn_child_process(first, 0, ProcessPriority::Normal, Credentials::system())
            .unwrap();

        let scheduled = kernel.kernel_schedule_next().unwrap();
        assert_eq!(scheduled.process, first);

        let next = kernel.kernel_yield_current(scheduled).unwrap().unwrap();
        assert_eq!(next.process, second);

        kernel.pending_mtss_decision = Some(next);
        assert_eq!(kernel.kernel_schedule_next().unwrap().process, second);
    }

    #[test]
    fn supervisor_starts_l2_before_device_daemons() {
        let mut kernel = boot_kernel();
        let supervisor = crate::supervisor::Supervisor::new();

        let report = supervisor.bootstrap_services(&mut kernel);

        assert!(report.all_running());
        let l2 = report
            .pid(crate::supervisor::ServiceId::L2Subkernel)
            .unwrap();
        assert_eq!(l2.raw(), 1);
        assert_eq!(
            report.state(crate::supervisor::ServiceId::Storaged),
            Some(crate::supervisor::StartupState::Running)
        );
        assert_eq!(
            report.state(crate::supervisor::ServiceId::Usbd),
            Some(crate::supervisor::StartupState::Running)
        );
        assert_eq!(
            report.state(crate::supervisor::ServiceId::Nvmed),
            Some(crate::supervisor::StartupState::Running)
        );
        assert_eq!(
            report.state(crate::supervisor::ServiceId::Ahcid),
            Some(crate::supervisor::StartupState::Running)
        );
        assert_eq!(
            report.state(crate::supervisor::ServiceId::Displayd),
            Some(crate::supervisor::StartupState::Running)
        );
        assert_eq!(
            report.state(crate::supervisor::ServiceId::AmdgpuDisplayd),
            Some(crate::supervisor::StartupState::Running)
        );
        assert_eq!(
            report.state(crate::supervisor::ServiceId::Networkd),
            Some(crate::supervisor::StartupState::Running)
        );
        assert_eq!(
            report.state(crate::supervisor::ServiceId::Inputd),
            Some(crate::supervisor::StartupState::Running)
        );

        let mut idx = 0usize;
        let mut child_count = 0usize;
        while idx < Kernel::<16, 4>::THREAD_CAPACITY {
            if idx < kernel.process_table.len() {
                if let Some(pcb) = kernel.process_table[idx] {
                    if pcb.pid != l2 {
                        assert_eq!(pcb.parent, Some(l2));
                        child_count += 1;
                    }
                }
            }
            idx += 1;
        }
        assert_eq!(child_count, 8);
    }

    #[test]
    fn receive_or_block_returns_queued_message_without_blocking() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let payload = MessagePayload::from_slice(SecurityClass::Public, b"ping");

        kernel.send_message(pid, pid, payload).unwrap();

        let message = kernel.receive_or_block(pid).unwrap().unwrap();
        assert_eq!(message.sender, pid);
        assert_eq!(message.receiver, pid);
        assert_eq!(&message.payload.data[..message.payload.length], b"ping");
        assert_eq!(process_state(&kernel, pid), ProcessState::Ready);
    }

    #[test]
    fn receive_or_block_atomically_blocks_empty_receiver() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();

        let message = kernel.receive_or_block(pid).unwrap();

        assert!(message.is_none());
        assert_eq!(process_state(&kernel, pid), ProcessState::Blocked);
        assert!(process_threads_blocked(&kernel, pid));
    }

    #[test]
    fn libc_receive_uses_blocking_receive_syscall() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let thread = first_thread(&kernel, pid);
        let mut out = Message::new(
            ProcessId::new(0),
            ProcessId::new(0),
            0,
            MessagePayload::empty(SecurityClass::Public),
        );

        let received =
            libc::receive_ipc_or_block(&mut kernel, pid, Some(thread), &mut out).unwrap();

        assert_eq!(received, None);
        assert_eq!(process_state(&kernel, pid), ProcessState::Blocked);
        assert!(process_threads_blocked(&kernel, pid));
    }

    #[test]
    fn security_errors_preserve_isolation_reason() {
        let mut kernel = boot_kernel();
        let init = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let user = kernel
            .spawn_child_process(init, 0, ProcessPriority::Normal, Credentials::user())
            .unwrap();
        let mut buffer = [0u8; 8];

        assert!(matches!(
            kernel.device_read(user, DeviceId::new(1), &mut buffer),
            Err(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            ))
        ));

        assert!(matches!(
            kernel.send_message(
                user,
                ProcessId::new(999),
                MessagePayload::empty(SecurityClass::Public)
            ),
            Err(KernelError::SecurityViolation(IsolationError::UnknownTask))
        ));
    }

    #[test]
    fn service_registry_routes_ipc_and_gates_raw_device_access() {
        let mut kernel = boot_kernel();
        let l2 = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let displayd = kernel
            .spawn_child_process(
                l2,
                0,
                ProcessPriority::High,
                Credentials::new(
                    SecurityLabel::internal(),
                    CapabilitySet::ipc_io(),
                    IsolationLevel::Process,
                ),
            )
            .unwrap();
        let user = kernel
            .spawn_child_process(l2, 0, ProcessPriority::Normal, Credentials::user())
            .unwrap();

        kernel
            .register_service(l2, RegistryServiceId::Displayd, displayd)
            .unwrap();

        let payload = MessagePayload::from_slice(SecurityClass::Public, b"draw");
        kernel
            .send_service_message(user, RegistryServiceId::Displayd, payload)
            .unwrap();

        let message = kernel.receive_message(displayd).unwrap();
        assert_eq!(message.sender, user);
        assert_eq!(message.receiver, displayd);
        assert_eq!(&message.payload.data[..message.payload.length], b"draw");

        let mut buffer = [0u8; 64];
        assert!(matches!(
            kernel.device_read(displayd, DeviceId::new(5), &mut buffer),
            Err(KernelError::SecurityViolation(
                IsolationError::PolicyViolation
            ))
        ));

        kernel
            .claim_service_device(displayd, RegistryServiceId::Displayd, DeviceId::new(5))
            .unwrap();
        assert!(kernel
            .device_read(displayd, DeviceId::new(5), &mut buffer)
            .is_ok());
    }

    #[test]
    fn kernel_exit_reports_without_registry_policy_cleanup() {
        let mut kernel = boot_kernel();
        let l2 = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let networkd = kernel
            .spawn_child_process(
                l2,
                0,
                ProcessPriority::High,
                Credentials::new(
                    SecurityLabel::internal(),
                    CapabilitySet::ipc_io(),
                    IsolationLevel::Process,
                ),
            )
            .unwrap();

        kernel
            .register_service(l2, RegistryServiceId::Networkd, networkd)
            .unwrap();
        kernel
            .claim_service_device(networkd, RegistryServiceId::Networkd, DeviceId::new(6))
            .unwrap();

        let exit = kernel
            .exit_process(networkd, ExitStatus::exited(0))
            .unwrap();

        assert_eq!(exit.pid, networkd);
        assert_eq!(exit.status.raw(), ExitStatus::exited(0).raw());
        assert_eq!(
            kernel.service_owner(RegistryServiceId::Networkd),
            Some(networkd)
        );
        assert!(matches!(
            kernel.device_write(networkd, DeviceId::new(6), b"ping"),
            Err(KernelError::SecurityViolation(IsolationError::UnknownTask))
        ));
    }

    #[test]
    fn syscall_error_encoding_maps_structured_security_reasons() {
        assert_eq!(
            encode_syscall_error(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            )),
            MIRAGE_SYSCALL_ERROR_BIT | SyscallErrorCode::PermissionDenied.raw()
        );
        assert_eq!(
            encode_syscall_error(KernelError::SecurityViolation(IsolationError::UnknownTask)),
            MIRAGE_SYSCALL_ERROR_BIT | SyscallErrorCode::NoSuchProcess.raw()
        );
        assert_eq!(
            encode_syscall_error(KernelError::MessageQueueFull),
            MIRAGE_SYSCALL_ERROR_BIT | SyscallErrorCode::QueueFull.raw()
        );
    }

    #[test]
    fn nanosleep_blocks_until_kernel_time_deadline() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let thread = first_thread(&kernel, pid);
        let req = MirageTimespec {
            tv_sec: 0,
            tv_nsec: 1,
        };

        kernel
            .handle_syscall(
                SyscallNumber::Nanosleep.raw(),
                SyscallContext::new(
                    pid,
                    Some(thread),
                    [&req as *const MirageTimespec as u64, 0, 0, 0, 0, 0],
                ),
            )
            .unwrap();

        assert_eq!(process_state(&kernel, pid), ProcessState::Blocked);
        assert!(process_threads_blocked(&kernel, pid));

        kernel.tick();

        assert_eq!(process_state(&kernel, pid), ProcessState::Ready);
        assert_eq!(
            kernel.thread_table[kernel.locate_thread(thread).unwrap()]
                .unwrap()
                .state,
            ThreadState::Ready
        );
    }

    #[test]
    fn tls_syscalls_record_fs_and_gs_bases() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let thread = first_thread(&kernel, pid);
        let mut out = 0u64;

        kernel
            .handle_syscall(
                SyscallNumber::SetThreadArea.raw(),
                SyscallContext::new(pid, Some(thread), [0x7000, 0, 0, 0, 0, 0]),
            )
            .unwrap();
        assert_eq!(kernel.thread_context(thread).unwrap().fs_base, 0x7000);

        kernel
            .handle_syscall(
                SyscallNumber::ArchPrctl.raw(),
                SyscallContext::new(pid, Some(thread), [ARCH_SET_FS, 0x8000, 0, 0, 0, 0]),
            )
            .unwrap();
        kernel
            .handle_syscall(
                SyscallNumber::ArchPrctl.raw(),
                SyscallContext::new(
                    pid,
                    Some(thread),
                    [ARCH_GET_FS, &mut out as *mut u64 as u64, 0, 0, 0, 0],
                ),
            )
            .unwrap();
        assert_eq!(out, 0x8000);

        kernel
            .handle_syscall(
                SyscallNumber::ArchPrctl.raw(),
                SyscallContext::new(pid, Some(thread), [ARCH_SET_GS, 0x9000, 0, 0, 0, 0]),
            )
            .unwrap();
        kernel
            .handle_syscall(
                SyscallNumber::ArchPrctl.raw(),
                SyscallContext::new(
                    pid,
                    Some(thread),
                    [ARCH_GET_GS, &mut out as *mut u64 as u64, 0, 0, 0, 0],
                ),
            )
            .unwrap();
        assert_eq!(out, 0x9000);
    }

    #[test]
    fn futex_wait_blocks_and_wake_requeues_thread() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let thread = first_thread(&kernel, pid);
        let word = 7i32;

        kernel
            .handle_syscall(
                SyscallNumber::Futex.raw(),
                SyscallContext::new(
                    pid,
                    Some(thread),
                    [&word as *const i32 as u64, FUTEX_WAIT, 7, 0, 0, 0],
                ),
            )
            .unwrap();
        assert_eq!(process_state(&kernel, pid), ProcessState::Blocked);
        assert_eq!(
            kernel.thread_table[kernel.locate_thread(thread).unwrap()]
                .unwrap()
                .state,
            ThreadState::Blocked
        );

        let woken = kernel
            .handle_syscall(
                SyscallNumber::Futex.raw(),
                SyscallContext::new(
                    pid,
                    None,
                    [&word as *const i32 as u64, FUTEX_WAKE, 1, 0, 0, 0],
                ),
            )
            .unwrap();

        assert_eq!(woken, 1);
        assert_eq!(process_state(&kernel, pid), ProcessState::Ready);
        assert_eq!(
            kernel.thread_table[kernel.locate_thread(thread).unwrap()]
                .unwrap()
                .state,
            ThreadState::Ready
        );
    }

    #[test]
    fn futex_wait_timeout_wakes_on_tick_with_timed_out_result() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let thread = first_thread(&kernel, pid);
        let word = 3i32;
        let timeout = MirageTimespec {
            tv_sec: 0,
            tv_nsec: 1,
        };

        kernel
            .handle_syscall(
                SyscallNumber::Futex.raw(),
                SyscallContext::new(
                    pid,
                    Some(thread),
                    [
                        &word as *const i32 as u64,
                        FUTEX_WAIT,
                        3,
                        &timeout as *const MirageTimespec as u64,
                        0,
                        0,
                    ],
                ),
            )
            .unwrap();

        kernel.tick();

        let context = kernel.thread_context(thread).unwrap();
        assert_eq!(process_state(&kernel, pid), ProcessState::Ready);
        assert_eq!(context.rax, encode_syscall_error(KernelError::TimedOut));
    }

    #[test]
    fn nanosleep_rejects_malformed_timespec() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let req = MirageTimespec {
            tv_sec: 0,
            tv_nsec: 1_000_000_000,
        };

        assert!(matches!(
            kernel.handle_syscall(
                SyscallNumber::Nanosleep.raw(),
                SyscallContext::new(
                    pid,
                    None,
                    [&req as *const MirageTimespec as u64, 0, 0, 0, 0, 0,],
                ),
            ),
            Err(KernelError::InvalidArgument)
        ));
        assert_eq!(process_state(&kernel, pid), ProcessState::Ready);
    }

    #[test]
    fn process_timers_are_owned_and_reject_bad_ids() {
        let mut kernel = boot_kernel();
        let owner = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let other = kernel
            .spawn_child_process(owner, 0, ProcessPriority::Normal, Credentials::user())
            .unwrap();
        let mut timer_id = 0u64;

        kernel
            .handle_syscall(
                SyscallNumber::TimerCreate.raw(),
                SyscallContext::new(
                    owner,
                    None,
                    [0, 0, &mut timer_id as *mut u64 as u64, 0, 0, 0],
                ),
            )
            .unwrap();
        assert_ne!(timer_id, 0);

        assert!(matches!(
            kernel.handle_syscall(
                SyscallNumber::TimerGettime.raw(),
                SyscallContext::new(
                    other,
                    None,
                    [
                        timer_id,
                        &mut MirageItimerspec {
                            it_interval: MirageTimespec {
                                tv_sec: 0,
                                tv_nsec: 0
                            },
                            it_value: MirageTimespec {
                                tv_sec: 0,
                                tv_nsec: 0
                            },
                        } as *mut MirageItimerspec as u64,
                        0,
                        0,
                        0,
                        0
                    ]
                ),
            ),
            Err(KernelError::InvalidArgument)
        ));
    }

    #[test]
    fn timer_settime_arms_gettime_and_delete() {
        let mut kernel = boot_kernel();
        let owner = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let mut timer_id = 0u64;
        let new_value = MirageItimerspec {
            it_interval: MirageTimespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: MirageTimespec {
                tv_sec: 0,
                tv_nsec: 1,
            },
        };
        let mut current = MirageItimerspec {
            it_interval: MirageTimespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: MirageTimespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
        };

        kernel
            .handle_syscall(
                SyscallNumber::TimerCreate.raw(),
                SyscallContext::new(
                    owner,
                    None,
                    [0, 0, &mut timer_id as *mut u64 as u64, 0, 0, 0],
                ),
            )
            .unwrap();
        kernel
            .handle_syscall(
                SyscallNumber::TimerSettime.raw(),
                SyscallContext::new(
                    owner,
                    None,
                    [
                        timer_id,
                        0,
                        &new_value as *const MirageItimerspec as u64,
                        0,
                        0,
                        0,
                    ],
                ),
            )
            .unwrap();
        kernel
            .handle_syscall(
                SyscallNumber::TimerGettime.raw(),
                SyscallContext::new(
                    owner,
                    None,
                    [
                        timer_id,
                        &mut current as *mut MirageItimerspec as u64,
                        0,
                        0,
                        0,
                        0,
                    ],
                ),
            )
            .unwrap();
        assert!(current.it_value.tv_sec > 0 || current.it_value.tv_nsec > 0);

        kernel
            .handle_syscall(
                SyscallNumber::TimerDelete.raw(),
                SyscallContext::new(owner, None, [timer_id, 0, 0, 0, 0, 0]),
            )
            .unwrap();
        assert!(matches!(
            kernel.handle_syscall(
                SyscallNumber::TimerDelete.raw(),
                SyscallContext::new(owner, None, [timer_id, 0, 0, 0, 0, 0]),
            ),
            Err(KernelError::InvalidArgument)
        ));
    }

    #[test]
    fn process_file_tables_share_inherited_open_descriptions() {
        let mut kernel = boot_kernel();
        let parent = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let path = b"/fdshare\0";
        let fd = kernel
            .handle_syscall(
                SyscallNumber::OpenAt.raw(),
                SyscallContext::new(
                    parent,
                    None,
                    [
                        AT_FDCWD as u64,
                        path.as_ptr() as u64,
                        (crate::kernel::fs::O_RDWR
                            | crate::kernel::fs::O_CREAT
                            | crate::kernel::fs::O_CLOEXEC) as u64,
                        0,
                        0,
                        0,
                    ],
                ),
            )
            .unwrap() as usize;
        let description = kernel.fd_description(parent, fd).unwrap();
        assert_eq!(kernel.open_files.ref_count(description).unwrap(), 1);
        assert!(kernel
            .process_files(parent)
            .unwrap()
            .get(fd)
            .unwrap()
            .close_on_exec());

        let child = kernel
            .spawn_child_process(parent, 0, ProcessPriority::Normal, Credentials::user())
            .unwrap();
        assert_eq!(kernel.fd_description(child, fd).unwrap(), description);
        assert_eq!(kernel.open_files.ref_count(description).unwrap(), 2);

        kernel
            .handle_syscall(
                SyscallNumber::Close.raw(),
                SyscallContext::new(parent, None, [fd as u64, 0, 0, 0, 0, 0]),
            )
            .unwrap();
        assert_eq!(kernel.open_files.ref_count(description).unwrap(), 1);

        kernel.terminate_process(child);
        assert!(matches!(
            kernel.open_files.ref_count(description),
            Err(FileTableError::InvalidDescriptor)
        ));
    }

    #[test]
    fn memory_syscalls_are_process_owned() {
        let mut kernel = boot_kernel();
        let owner = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let other = kernel
            .spawn_child_process(owner, 0, ProcessPriority::Normal, Credentials::user())
            .unwrap();
        let ptr = libc::malloc(&mut kernel, owner, None, 64).unwrap();

        assert!(matches!(
            libc::free(&mut kernel, other, None, ptr),
            Err(KernelError::InvalidArgument)
        ));
        assert!(libc::free(&mut kernel, owner, None, ptr).is_ok());
    }

    #[test]
    fn mmap_rejects_writable_executable_mapping() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let protection = MemoryProtection::from_bits(PROT_READ | PROT_WRITE | PROT_EXECUTE);

        assert!(matches!(
            libc::mmap(&mut kernel, pid, None, 4096, protection),
            Err(KernelError::SecurityViolation(_))
        ));
    }

    #[test]
    fn mmap_rejects_executable_mapping_for_unprivileged_process() {
        let mut kernel = boot_kernel();
        let init = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let user = kernel
            .spawn_child_process(init, 0, ProcessPriority::Normal, Credentials::user())
            .unwrap();
        let protection = MemoryProtection::from_bits(PROT_READ | PROT_EXECUTE);

        assert!(matches!(
            libc::mmap(&mut kernel, user, None, 4096, protection),
            Err(KernelError::SecurityViolation(_))
        ));
    }
}

fn wait_selector_matches(
    selector: i64,
    child_pid: ProcessId,
    child_pgid: ProcessGroupId,
    parent_pgid: ProcessGroupId,
) -> bool {
    if selector == -1 {
        true
    } else if selector == 0 {
        child_pgid == parent_pgid
    } else if selector > 0 {
        child_pid.raw() == selector as u64
    } else {
        child_pgid.raw() == (-selector) as u64
    }
}

fn push_user_u64(
    kernel_base: *mut u8,
    user_base: u64,
    sp: &mut u64,
    value: u64,
) -> Result<(), crate::kernel::userspace::LoadError> {
    *sp = sp
        .checked_sub(core::mem::size_of::<u64>() as u64)
        .ok_or(crate::kernel::userspace::LoadError::StackBuildFailed)?;
    let offset = sp
        .checked_sub(user_base)
        .ok_or(crate::kernel::userspace::LoadError::StackBuildFailed)? as usize;
    unsafe {
        core::ptr::copy_nonoverlapping(
            value.to_le_bytes().as_ptr(),
            kernel_base.add(offset),
            core::mem::size_of::<u64>(),
        );
    }
    Ok(())
}

fn map_mtss_error(error: MtssError) -> KernelError {
    match error {
        MtssError::RunQueueFull => KernelError::SchedulerFull,
        MtssError::EmptyRunQueue => KernelError::UnknownThread,
        MtssError::InvalidTask => KernelError::UnknownProcess,
        MtssError::InvalidThread => KernelError::UnknownThread,
        MtssError::TaskTableFull => KernelError::ProcessTableFull,
        MtssError::ThreadTableFull => KernelError::ThreadTableFull,
        MtssError::InvalidTaskTransition { .. }
        | MtssError::InvalidThreadTransition { .. }
        | MtssError::AlreadyCurrent => KernelError::InvalidArgument,
        MtssError::BackendUnavailable => KernelError::DeviceFault(DriverError::Unsupported),
        MtssError::CapabilityDenied => {
            KernelError::SecurityViolation(IsolationError::CapabilityMissing)
        }
    }
}

fn map_core_mtss_error(error: CoreMtssError) -> KernelError {
    match error {
        CoreMtssError::TaskTableFull => KernelError::ProcessTableFull,
        CoreMtssError::ThreadTableFull => KernelError::ThreadTableFull,
        CoreMtssError::ReadyQueueFull => KernelError::SchedulerFull,
        CoreMtssError::InvalidAddressSpace
        | CoreMtssError::InvalidEntry
        | CoreMtssError::InvalidStack => KernelError::InvalidArgument,
        CoreMtssError::UnknownTask => KernelError::UnknownProcess,
        CoreMtssError::UnknownThread | CoreMtssError::ReadyQueueEmpty => KernelError::UnknownThread,
    }
}
