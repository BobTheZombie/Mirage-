//! errno storage and Mirage error translations.

use crate::kernel::fs::errno_from_vfs;
use crate::kernel::syscall::{
    MIRAGE_EACCES, MIRAGE_EAGAIN, MIRAGE_EFAULT, MIRAGE_EINVAL, MIRAGE_EIO, MIRAGE_ENOBUFS,
    MIRAGE_ENOMEM, MIRAGE_ENOSYS, MIRAGE_ESRCH,
};
use crate::kernel::KernelError;
use crate::subkernel::IsolationError;

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
        KernelError::DeviceNotFound => MIRAGE_ESRCH,
        KernelError::DeviceFault(_) => MIRAGE_EIO,
        KernelError::InvalidSyscall => MIRAGE_ENOSYS,
        KernelError::InvalidArgument => MIRAGE_EINVAL,
        KernelError::InvalidPointer => MIRAGE_EFAULT,
        KernelError::Filesystem(error) => libc_vfs_errno(error),
    }
}

fn libc_vfs_errno(error: crate::kernel::fs::VfsError) -> i32 {
    errno_from_vfs(error)
}
