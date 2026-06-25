//! Stable system call ABI shared by user-facing wrappers and the kernel.
//!
//! The table in [`SyscallNumber`] is append-only: existing numeric assignments
//! are treated as ABI and must not be reused for a different operation.

use crate::kernel::memory::{self, MemoryProtection};
use crate::kernel::process::ProcessId;
use crate::kernel::thread::{CpuContext, ThreadId};

pub use mirage_abi::syscall::*;

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
