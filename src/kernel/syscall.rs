//! Stable system call ABI shared by user-facing wrappers and the kernel.
//!
//! The table in [`SyscallNumber`] is append-only: existing numeric assignments
//! are treated as ABI and must not be reused for a different operation.

use crate::kernel::memory::{self, MemoryProtection};
use crate::kernel::process::ProcessId;
use crate::kernel::thread::{CpuContext, ThreadId};

pub const SYSCALL_MAX_ARGS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallFrame {
    pub number: u64,
    pub args: [u64; SYSCALL_MAX_ARGS],
}

impl SyscallFrame {
    pub const fn from_cpu_context(context: &CpuContext) -> Self {
        Self {
            number: context.syscall_number(),
            args: context.syscall_args(),
        }
    }
}

/// High bit set on a trap return value means the low bits carry a
/// [`SyscallErrorCode`] instead of a successful result.
pub const MIRAGE_SYSCALL_ERROR_BIT: u64 = 1 << 63;

/// Stable syscall error numbers exposed in encoded trap results.
///
/// Libc-style wrappers translate these to negative errno values at the C ABI:
/// capability-missing and policy-denied security failures become `EACCES`,
/// unknown tasks/processes become `ESRCH`, and full IPC queues become
/// `ENOBUFS` so callers can distinguish back-pressure from invalid input.
#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyscallErrorCode {
    ProcessTableFull = 1,
    SchedulerFull = 2,
    NoSuchProcess = 3,
    NoSuchThread = 4,
    ThreadTableFull = 5,
    QueueFull = 6,
    QueueEmpty = 7,
    PermissionDenied = 8,
    IsolationFault = 9,
    NoSuchDevice = 10,
    DeviceFault = 11,
    InvalidSyscall = 12,
    InvalidArgument = 13,
    BadAddress = 14,
    OutOfMemory = 15,
    FileNotFound = 16,
    BadFileDescriptor = 17,
    NotDirectory = 18,
    IsDirectory = 19,
    AlreadyExists = 20,
    ReadOnlyFilesystem = 21,
    NoSpace = 22,
    FilesystemBusy = 23,
    CrossDevice = 24,
    TooManyLinks = 25,
    UnsupportedFilesystem = 26,
    NameTooLong = 27,
}

impl SyscallErrorCode {
    pub const fn raw(self) -> u64 {
        self as u64
    }

    pub const fn linux_errno(self) -> i32 {
        match self {
            Self::ProcessTableFull | Self::SchedulerFull | Self::ThreadTableFull => MIRAGE_ENOMEM,
            Self::NoSuchProcess | Self::NoSuchThread => MIRAGE_ESRCH,
            Self::QueueFull => MIRAGE_ENOBUFS,
            Self::QueueEmpty => MIRAGE_EAGAIN,
            Self::PermissionDenied | Self::IsolationFault => MIRAGE_EACCES,
            Self::NoSuchDevice => MIRAGE_ENODEV,
            Self::DeviceFault => MIRAGE_EIO,
            Self::InvalidSyscall => MIRAGE_ENOSYS,
            Self::InvalidArgument => MIRAGE_EINVAL,
            Self::BadAddress => MIRAGE_EFAULT,
            Self::OutOfMemory => MIRAGE_ENOMEM,
            Self::FileNotFound => MIRAGE_ENOENT,
            Self::BadFileDescriptor => MIRAGE_EBADF,
            Self::NotDirectory => MIRAGE_ENOTDIR,
            Self::IsDirectory => MIRAGE_EISDIR,
            Self::AlreadyExists => MIRAGE_EEXIST,
            Self::ReadOnlyFilesystem => MIRAGE_EROFS,
            Self::NoSpace => MIRAGE_ENOSPC,
            Self::FilesystemBusy => MIRAGE_EBUSY,
            Self::CrossDevice => MIRAGE_EXDEV,
            Self::TooManyLinks => MIRAGE_EMLINK,
            Self::UnsupportedFilesystem => MIRAGE_ENOTSUP,
            Self::NameTooLong => MIRAGE_ENAMETOOLONG,
        }
    }
}

// Minimal errno values used by the exported C-facing libc shims.  They match
// the Linux errno assignments for the Unix-like errors Mirage currently emits.
pub const MIRAGE_EPERM: i32 = 1;
pub const MIRAGE_ENOENT: i32 = 2;
pub const MIRAGE_ESRCH: i32 = 3;
pub const MIRAGE_EINTR: i32 = 4;
pub const MIRAGE_EIO: i32 = 5;
pub const MIRAGE_EBADF: i32 = 9;
pub const MIRAGE_EAGAIN: i32 = 11;
pub const MIRAGE_ENOMEM: i32 = 12;
pub const MIRAGE_EACCES: i32 = 13;
pub const MIRAGE_EFAULT: i32 = 14;
pub const MIRAGE_EBUSY: i32 = 16;
pub const MIRAGE_EEXIST: i32 = 17;
pub const MIRAGE_EXDEV: i32 = 18;
pub const MIRAGE_ENODEV: i32 = 19;
pub const MIRAGE_ENOTDIR: i32 = 20;
pub const MIRAGE_EISDIR: i32 = 21;
pub const MIRAGE_EINVAL: i32 = 22;
pub const MIRAGE_ENOSPC: i32 = 28;
pub const MIRAGE_EROFS: i32 = 30;
pub const MIRAGE_EMLINK: i32 = 31;
pub const MIRAGE_ENAMETOOLONG: i32 = 36;
pub const MIRAGE_ENOSYS: i32 = 38;
pub const MIRAGE_ENOTSUP: i32 = 95;
pub const MIRAGE_ENOBUFS: i32 = 105;

#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyscallNumber {
    GetPid = 0,
    Spawn = 1,
    SendIpc = 2,
    ReceiveIpc = 3,
    BlockForIpc = 4,
    EnumerateDevices = 5,
    DeviceRead = 6,
    DeviceWrite = 7,
    Mmap = 8,
    Munmap = 9,
    Malloc = 10,
    Free = 11,
    ReceiveOrBlockIpc = 12,
    Realloc = 13,
    MallocAligned = 14,
    DeviceInfo = 15,
    OpenAt = 16,
    Close = 17,
    Read = 18,
    Write = 19,
    Pread64 = 20,
    Pwrite64 = 21,
    Lseek = 22,
    Statx = 23,
    NewFstatAt = 24,
    Getdents64 = 25,
    MkdirAt = 26,
    UnlinkAt = 27,
    RenameAt2 = 28,
    Ftruncate = 29,
    Fsync = 30,
    Mount = 31,
}

impl SyscallNumber {
    pub const fn raw(self) -> u64 {
        self as u64
    }

    pub const fn from_raw(raw: u64) -> Option<Self> {
        match raw {
            0 => Some(Self::GetPid),
            1 => Some(Self::Spawn),
            2 => Some(Self::SendIpc),
            3 => Some(Self::ReceiveIpc),
            4 => Some(Self::BlockForIpc),
            5 => Some(Self::EnumerateDevices),
            6 => Some(Self::DeviceRead),
            7 => Some(Self::DeviceWrite),
            8 => Some(Self::Mmap),
            9 => Some(Self::Munmap),
            10 => Some(Self::Malloc),
            11 => Some(Self::Free),
            12 => Some(Self::ReceiveOrBlockIpc),
            13 => Some(Self::Realloc),
            14 => Some(Self::MallocAligned),
            15 => Some(Self::DeviceInfo),
            16 => Some(Self::OpenAt),
            17 => Some(Self::Close),
            18 => Some(Self::Read),
            19 => Some(Self::Write),
            20 => Some(Self::Pread64),
            21 => Some(Self::Pwrite64),
            22 => Some(Self::Lseek),
            23 => Some(Self::Statx),
            24 => Some(Self::NewFstatAt),
            25 => Some(Self::Getdents64),
            26 => Some(Self::MkdirAt),
            27 => Some(Self::UnlinkAt),
            28 => Some(Self::RenameAt2),
            29 => Some(Self::Ftruncate),
            30 => Some(Self::Fsync),
            31 => Some(Self::Mount),
            _ => None,
        }
    }
}

pub const MIRAGE_SYSCALL_GETPID: u64 = SyscallNumber::GetPid.raw();
pub const MIRAGE_SYSCALL_SPAWN: u64 = SyscallNumber::Spawn.raw();
pub const MIRAGE_SYSCALL_SEND_IPC: u64 = SyscallNumber::SendIpc.raw();
pub const MIRAGE_SYSCALL_RECEIVE_IPC: u64 = SyscallNumber::ReceiveIpc.raw();
pub const MIRAGE_SYSCALL_BLOCK_FOR_IPC: u64 = SyscallNumber::BlockForIpc.raw();
pub const MIRAGE_SYSCALL_ENUMERATE_DEVICES: u64 = SyscallNumber::EnumerateDevices.raw();
pub const MIRAGE_SYSCALL_DEVICE_READ: u64 = SyscallNumber::DeviceRead.raw();
pub const MIRAGE_SYSCALL_DEVICE_WRITE: u64 = SyscallNumber::DeviceWrite.raw();
pub const MIRAGE_SYSCALL_DEVICE_INFO: u64 = SyscallNumber::DeviceInfo.raw();
pub const MIRAGE_SYSCALL_OPENAT: u64 = SyscallNumber::OpenAt.raw();
pub const MIRAGE_SYSCALL_CLOSE: u64 = SyscallNumber::Close.raw();
pub const MIRAGE_SYSCALL_READ: u64 = SyscallNumber::Read.raw();
pub const MIRAGE_SYSCALL_WRITE: u64 = SyscallNumber::Write.raw();
pub const MIRAGE_SYSCALL_PREAD64: u64 = SyscallNumber::Pread64.raw();
pub const MIRAGE_SYSCALL_PWRITE64: u64 = SyscallNumber::Pwrite64.raw();
pub const MIRAGE_SYSCALL_LSEEK: u64 = SyscallNumber::Lseek.raw();
pub const MIRAGE_SYSCALL_STATX: u64 = SyscallNumber::Statx.raw();
pub const MIRAGE_SYSCALL_NEWFSTATAT: u64 = SyscallNumber::NewFstatAt.raw();
pub const MIRAGE_SYSCALL_GETDENTS64: u64 = SyscallNumber::Getdents64.raw();
pub const MIRAGE_SYSCALL_MKDIRAT: u64 = SyscallNumber::MkdirAt.raw();
pub const MIRAGE_SYSCALL_UNLINKAT: u64 = SyscallNumber::UnlinkAt.raw();
pub const MIRAGE_SYSCALL_RENAMEAT2: u64 = SyscallNumber::RenameAt2.raw();
pub const MIRAGE_SYSCALL_FTRUNCATE: u64 = SyscallNumber::Ftruncate.raw();
pub const MIRAGE_SYSCALL_FSYNC: u64 = SyscallNumber::Fsync.raw();
pub const MIRAGE_SYSCALL_MOUNT: u64 = SyscallNumber::Mount.raw();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallContext {
    pub caller: ProcessId,
    pub thread: Option<ThreadId>,
    pub args: [u64; SYSCALL_MAX_ARGS],
}

impl SyscallContext {
    pub const fn new(
        caller: ProcessId,
        thread: Option<ThreadId>,
        args: [u64; SYSCALL_MAX_ARGS],
    ) -> Self {
        Self {
            caller,
            thread,
            args,
        }
    }

    pub const fn arg(&self, index: usize) -> u64 {
        self.args[index]
    }
}

/// Dispatches kernel-internal memory requests through the same syscall ABI shape
/// used by user traps. This is used by runtime shims that cannot carry a full
/// [`Kernel`](crate::kernel::Kernel) reference but still need allocations to be
/// attributed to a caller instead of bypassing process-aware memory accounting.
pub fn dispatch_kernel_memory_syscall(number: SyscallNumber, context: SyscallContext) -> u64 {
    match number {
        SyscallNumber::Mmap => {
            let length = context.arg(0) as usize;
            let protection = MemoryProtection::from_bits(context.arg(1) as u32);
            memory::mmap_for(context.caller, length, protection)
                .map(|region| region.as_ptr() as u64)
                .unwrap_or(0)
        }
        SyscallNumber::Munmap => {
            let Some(ptr) = core::ptr::NonNull::new(context.arg(0) as *mut u8) else {
                return u64::MAX;
            };
            if memory::munmap_ptr_for(context.caller, ptr, context.arg(1) as usize) {
                0
            } else {
                u64::MAX
            }
        }
        SyscallNumber::Malloc => memory::malloc_for(context.caller, context.arg(0) as usize)
            .map(|ptr| ptr.as_ptr() as u64)
            .unwrap_or(0),
        SyscallNumber::Free => {
            let Some(ptr) = core::ptr::NonNull::new(context.arg(0) as *mut u8) else {
                return 0;
            };
            if memory::free_for(context.caller, ptr) {
                0
            } else {
                u64::MAX
            }
        }
        SyscallNumber::Realloc => {
            let ptr = core::ptr::NonNull::new(context.arg(0) as *mut u8);
            memory::realloc_for(context.caller, ptr, context.arg(1) as usize)
                .map(|ptr| ptr.as_ptr() as u64)
                .unwrap_or(0)
        }
        SyscallNumber::MallocAligned => memory::malloc_aligned_for(
            context.caller,
            context.arg(0) as usize,
            context.arg(1) as usize,
        )
        .map(|ptr| ptr.as_ptr() as u64)
        .unwrap_or(0),
        _ => u64::MAX,
    }
}
