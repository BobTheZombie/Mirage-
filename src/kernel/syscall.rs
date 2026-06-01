//! Stable system call ABI shared by user-facing wrappers and the kernel.
//!
//! The table in [`SyscallNumber`] is append-only: existing numeric assignments
//! are treated as ABI and must not be reused for a different operation.

use crate::kernel::process::ProcessId;
use crate::kernel::thread::ThreadId;

pub const SYSCALL_MAX_ARGS: usize = 6;

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
            _ => None,
        }
    }
}

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
