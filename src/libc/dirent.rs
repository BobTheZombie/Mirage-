//! Directory entry C ABI wrappers.
#![allow(dead_code)]

use crate::kernel::fs::CDirEntry;
use crate::kernel::syscall::{self, MIRAGE_EFAULT};

use super::{errno, raw_syscall_default, raw_syscall_runtime, DefaultKernel};

pub(crate) unsafe fn mirage_getdents64_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    entries: *mut CDirEntry,
    count: usize,
) -> isize {
    if count > 0 && entries.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETDENTS64,
        [fd as u64, entries as u64, count as u64, 0, 0, 0],
    )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getdents64(fd: i32, entries: *mut CDirEntry, count: usize) -> isize {
    if count > 0 && entries.is_null() {
        return errno::return_or_errno(-(MIRAGE_EFAULT as isize));
    }
    errno::return_or_errno(raw_syscall_runtime(
        syscall::MIRAGE_SYSCALL_GETDENTS64,
        [fd as u64, entries as u64, count as u64, 0, 0, 0],
    ))
}
