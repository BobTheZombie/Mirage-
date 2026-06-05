//! Time-related C ABI wrappers.
#![allow(dead_code)]

use crate::kernel::syscall::{self, MIRAGE_EFAULT};
use crate::kernel::MirageTimespec;

use super::{errno, raw_syscall_default, raw_syscall_runtime, DefaultKernel};

pub(crate) unsafe fn mirage_clock_gettime_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    clock_id: i32,
    out: *mut MirageTimespec,
) -> isize {
    if out.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_CLOCK_GETTIME,
        [clock_id as u64, out as u64, 0, 0, 0, 0],
    )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn clock_gettime(clock_id: i32, out: *mut MirageTimespec) -> isize {
    if out.is_null() {
        return errno::return_or_errno(-(MIRAGE_EFAULT as isize));
    }
    errno::return_or_errno(raw_syscall_runtime(
        syscall::MIRAGE_SYSCALL_CLOCK_GETTIME,
        [clock_id as u64, out as u64, 0, 0, 0, 0],
    ))
}
