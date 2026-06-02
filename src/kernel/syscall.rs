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
    Chdir = 32,
    Fchdir = 33,
    Getcwd = 34,
    Faccessat = 35,
    Fchmodat = 36,
    Fchownat = 37,
    Symlinkat = 38,
    Readlinkat = 39,
    Linkat = 40,

    // Process lifecycle syscalls (100-119).
    Fork = 100,
    Execve = 101,
    Exit = 102,
    Wait4 = 103,
    GetPpid = 104,
    SetPgid = 105,
    Setsid = 106,

    // Credential syscalls (120-139).
    GetUid = 120,
    GetEuid = 121,
    SetUid = 122,
    GetGid = 123,
    SetGid = 124,
    GetGroups = 125,
    SetGroups = 126,

    // Signal syscalls (140-159).
    RtSigaction = 140,
    RtSigprocmask = 141,
    Kill = 142,
    RtSigreturn = 143,

    // Time syscalls (160-179).
    ClockGettime = 160,
    Nanosleep = 161,
    TimerCreate = 162,
    TimerSettime = 163,
    TimerGettime = 164,
    TimerDelete = 165,

    // Descriptor syscalls (180-199).
    Dup = 180,
    Dup2 = 181,
    Dup3 = 182,
    Fcntl = 183,
    Ioctl = 184,

    // Pipe/event syscalls (200-219).
    Pipe2 = 200,
    Poll = 201,
    Pselect = 202,
    Eventfd = 203,

    // Networking syscalls (220-239).
    Socket = 220,
    Bind = 221,
    Listen = 222,
    Accept = 223,
    Connect = 224,
    Sendmsg = 225,
    Recvmsg = 226,

    // Thread/TLS syscalls (240-259).
    Clone = 240,
    Futex = 241,
    SetThreadArea = 242,
    ArchPrctl = 243,
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
            32 => Some(Self::Chdir),
            33 => Some(Self::Fchdir),
            34 => Some(Self::Getcwd),
            35 => Some(Self::Faccessat),
            36 => Some(Self::Fchmodat),
            37 => Some(Self::Fchownat),
            38 => Some(Self::Symlinkat),
            39 => Some(Self::Readlinkat),
            40 => Some(Self::Linkat),
            100 => Some(Self::Fork),
            101 => Some(Self::Execve),
            102 => Some(Self::Exit),
            103 => Some(Self::Wait4),
            104 => Some(Self::GetPpid),
            105 => Some(Self::SetPgid),
            106 => Some(Self::Setsid),
            120 => Some(Self::GetUid),
            121 => Some(Self::GetEuid),
            122 => Some(Self::SetUid),
            123 => Some(Self::GetGid),
            124 => Some(Self::SetGid),
            125 => Some(Self::GetGroups),
            126 => Some(Self::SetGroups),
            140 => Some(Self::RtSigaction),
            141 => Some(Self::RtSigprocmask),
            142 => Some(Self::Kill),
            143 => Some(Self::RtSigreturn),
            160 => Some(Self::ClockGettime),
            161 => Some(Self::Nanosleep),
            162 => Some(Self::TimerCreate),
            163 => Some(Self::TimerSettime),
            164 => Some(Self::TimerGettime),
            165 => Some(Self::TimerDelete),
            180 => Some(Self::Dup),
            181 => Some(Self::Dup2),
            182 => Some(Self::Dup3),
            183 => Some(Self::Fcntl),
            184 => Some(Self::Ioctl),
            200 => Some(Self::Pipe2),
            201 => Some(Self::Poll),
            202 => Some(Self::Pselect),
            203 => Some(Self::Eventfd),
            220 => Some(Self::Socket),
            221 => Some(Self::Bind),
            222 => Some(Self::Listen),
            223 => Some(Self::Accept),
            224 => Some(Self::Connect),
            225 => Some(Self::Sendmsg),
            226 => Some(Self::Recvmsg),
            240 => Some(Self::Clone),
            241 => Some(Self::Futex),
            242 => Some(Self::SetThreadArea),
            243 => Some(Self::ArchPrctl),
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
pub const MIRAGE_SYSCALL_CHDIR: u64 = SyscallNumber::Chdir.raw();
pub const MIRAGE_SYSCALL_FCHDIR: u64 = SyscallNumber::Fchdir.raw();
pub const MIRAGE_SYSCALL_GETCWD: u64 = SyscallNumber::Getcwd.raw();
pub const MIRAGE_SYSCALL_FACCESSAT: u64 = SyscallNumber::Faccessat.raw();
pub const MIRAGE_SYSCALL_FCHMODAT: u64 = SyscallNumber::Fchmodat.raw();
pub const MIRAGE_SYSCALL_FCHOWNAT: u64 = SyscallNumber::Fchownat.raw();
pub const MIRAGE_SYSCALL_SYMLINKAT: u64 = SyscallNumber::Symlinkat.raw();
pub const MIRAGE_SYSCALL_READLINKAT: u64 = SyscallNumber::Readlinkat.raw();
pub const MIRAGE_SYSCALL_LINKAT: u64 = SyscallNumber::Linkat.raw();

pub const MIRAGE_SYSCALL_FORK: u64 = SyscallNumber::Fork.raw();
pub const MIRAGE_SYSCALL_EXECVE: u64 = SyscallNumber::Execve.raw();
pub const MIRAGE_SYSCALL_EXIT: u64 = SyscallNumber::Exit.raw();
pub const MIRAGE_SYSCALL_WAIT4: u64 = SyscallNumber::Wait4.raw();
pub const MIRAGE_SYSCALL_GETPPID: u64 = SyscallNumber::GetPpid.raw();
pub const MIRAGE_SYSCALL_SETPGID: u64 = SyscallNumber::SetPgid.raw();
pub const MIRAGE_SYSCALL_SETSID: u64 = SyscallNumber::Setsid.raw();
pub const MIRAGE_SYSCALL_GETUID: u64 = SyscallNumber::GetUid.raw();
pub const MIRAGE_SYSCALL_GETEUID: u64 = SyscallNumber::GetEuid.raw();
pub const MIRAGE_SYSCALL_SETUID: u64 = SyscallNumber::SetUid.raw();
pub const MIRAGE_SYSCALL_GETGID: u64 = SyscallNumber::GetGid.raw();
pub const MIRAGE_SYSCALL_SETGID: u64 = SyscallNumber::SetGid.raw();
pub const MIRAGE_SYSCALL_GETGROUPS: u64 = SyscallNumber::GetGroups.raw();
pub const MIRAGE_SYSCALL_SETGROUPS: u64 = SyscallNumber::SetGroups.raw();
pub const MIRAGE_SYSCALL_RT_SIGACTION: u64 = SyscallNumber::RtSigaction.raw();
pub const MIRAGE_SYSCALL_RT_SIGPROCMASK: u64 = SyscallNumber::RtSigprocmask.raw();
pub const MIRAGE_SYSCALL_KILL: u64 = SyscallNumber::Kill.raw();
pub const MIRAGE_SYSCALL_RT_SIGRETURN: u64 = SyscallNumber::RtSigreturn.raw();
pub const MIRAGE_SYSCALL_CLOCK_GETTIME: u64 = SyscallNumber::ClockGettime.raw();
pub const MIRAGE_SYSCALL_NANOSLEEP: u64 = SyscallNumber::Nanosleep.raw();
pub const MIRAGE_SYSCALL_TIMER_CREATE: u64 = SyscallNumber::TimerCreate.raw();
pub const MIRAGE_SYSCALL_TIMER_SETTIME: u64 = SyscallNumber::TimerSettime.raw();
pub const MIRAGE_SYSCALL_TIMER_GETTIME: u64 = SyscallNumber::TimerGettime.raw();
pub const MIRAGE_SYSCALL_TIMER_DELETE: u64 = SyscallNumber::TimerDelete.raw();
pub const MIRAGE_SYSCALL_DUP: u64 = SyscallNumber::Dup.raw();
pub const MIRAGE_SYSCALL_DUP2: u64 = SyscallNumber::Dup2.raw();
pub const MIRAGE_SYSCALL_DUP3: u64 = SyscallNumber::Dup3.raw();
pub const MIRAGE_SYSCALL_FCNTL: u64 = SyscallNumber::Fcntl.raw();
pub const MIRAGE_SYSCALL_IOCTL: u64 = SyscallNumber::Ioctl.raw();
pub const MIRAGE_SYSCALL_PIPE2: u64 = SyscallNumber::Pipe2.raw();
pub const MIRAGE_SYSCALL_POLL: u64 = SyscallNumber::Poll.raw();
pub const MIRAGE_SYSCALL_PSELECT: u64 = SyscallNumber::Pselect.raw();
pub const MIRAGE_SYSCALL_EVENTFD: u64 = SyscallNumber::Eventfd.raw();
pub const MIRAGE_SYSCALL_SOCKET: u64 = SyscallNumber::Socket.raw();
pub const MIRAGE_SYSCALL_BIND: u64 = SyscallNumber::Bind.raw();
pub const MIRAGE_SYSCALL_LISTEN: u64 = SyscallNumber::Listen.raw();
pub const MIRAGE_SYSCALL_ACCEPT: u64 = SyscallNumber::Accept.raw();
pub const MIRAGE_SYSCALL_CONNECT: u64 = SyscallNumber::Connect.raw();
pub const MIRAGE_SYSCALL_SENDMSG: u64 = SyscallNumber::Sendmsg.raw();
pub const MIRAGE_SYSCALL_RECVMSG: u64 = SyscallNumber::Recvmsg.raw();
pub const MIRAGE_SYSCALL_CLONE: u64 = SyscallNumber::Clone.raw();
pub const MIRAGE_SYSCALL_FUTEX: u64 = SyscallNumber::Futex.raw();
pub const MIRAGE_SYSCALL_SET_THREAD_AREA: u64 = SyscallNumber::SetThreadArea.raw();
pub const MIRAGE_SYSCALL_ARCH_PRCTL: u64 = SyscallNumber::ArchPrctl.raw();

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
