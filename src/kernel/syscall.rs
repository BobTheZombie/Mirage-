//! Stable system call ABI shared by user-facing wrappers and the kernel.
//!
//! The table in [`SyscallNumber`] is append-only: existing numeric assignments
//! are treated as ABI and must not be reused for a different operation.

use crate::kernel::memory::{self, MemoryProtection};
use crate::kernel::process::ProcessId;
use crate::kernel::thread::ThreadId;

pub const SYSCALL_MAX_ARGS: usize = 6;

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
}

impl SyscallErrorCode {
    pub const fn raw(self) -> u64 {
        self as u64
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
pub const MIRAGE_EINVAL: i32 = 22;
pub const MIRAGE_ENOSYS: i32 = 38;
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
