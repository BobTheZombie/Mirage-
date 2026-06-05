//! errno storage and Mirage error translations.

use crate::kernel::fs::errno_from_vfs;
use crate::kernel::syscall::{
    MIRAGE_EACCES, MIRAGE_EAGAIN, MIRAGE_EFAULT, MIRAGE_EINVAL, MIRAGE_EIO, MIRAGE_ENOBUFS,
    MIRAGE_ENOMEM, MIRAGE_ENOSYS, MIRAGE_ESRCH, MIRAGE_ETIMEDOUT,
};
use crate::kernel::KernelError;
use crate::subkernel::IsolationError;

/// Process-wide errno storage exported for C sysroot headers.
///
/// Mirage currently runs these libc shims in a single kernel/runtime context, so
/// errno is process-wide until TLS-backed per-thread errno is wired into the
/// userspace runtime.
#[cfg_attr(not(test), no_mangle)]
pub static mut errno: i32 = 0;

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn __errno_location() -> *mut i32 {
    core::ptr::addr_of_mut!(errno)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn mirage_errno_location() -> *mut i32 {
    __errno_location()
}

pub(super) fn set_errno(value: i32) {
    unsafe {
        errno = value;
    }
}

pub(super) fn return_or_errno(value: isize) -> isize {
    if value < 0 {
        set_errno((-value) as i32);
        -1
    } else {
        value
    }
}

pub(super) fn libc_errno(error: KernelError) -> i32 {
    match error {
        KernelError::ProcessTableFull
        | KernelError::SchedulerFull
        | KernelError::ThreadTableFull
        | KernelError::AllocationFailed
        | KernelError::FileTableFull => MIRAGE_ENOMEM,
        KernelError::UnknownProcess | KernelError::UnknownThread => MIRAGE_ESRCH,
        KernelError::MessageQueueFull => MIRAGE_ENOBUFS,
        KernelError::MessageQueueEmpty => MIRAGE_EAGAIN,
        KernelError::SecurityViolation(IsolationError::UnknownTask)
        | KernelError::IsolationFault(IsolationError::UnknownTask) => MIRAGE_ESRCH,
        KernelError::SecurityViolation(
            IsolationError::CapabilityMissing | IsolationError::PolicyViolation,
        )
        | KernelError::IsolationFault(
            IsolationError::CapabilityMissing | IsolationError::PolicyViolation,
        ) => MIRAGE_EACCES,
        KernelError::SecurityViolation(IsolationError::CapabilityTableFull)
        | KernelError::IsolationFault(IsolationError::CapabilityTableFull) => MIRAGE_ENOMEM,
        KernelError::DeviceNotFound => MIRAGE_ESRCH,
        KernelError::DeviceFault(_) => MIRAGE_EIO,
        KernelError::InvalidSyscall => MIRAGE_ENOSYS,
        KernelError::InvalidArgument => MIRAGE_EINVAL,
        KernelError::InvalidPointer => MIRAGE_EFAULT,
        KernelError::TimedOut => MIRAGE_ETIMEDOUT,
        KernelError::Filesystem(error) => libc_vfs_errno(error),
    }
}

fn libc_vfs_errno(error: crate::kernel::fs::VfsError) -> i32 {
    errno_from_vfs(error)
}
