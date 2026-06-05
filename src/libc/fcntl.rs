//! Open and descriptor flag C ABI exports.
#![allow(dead_code)]

use crate::kernel::syscall::{self, MIRAGE_EFAULT};

use super::{errno, raw_syscall_default, raw_syscall_runtime, DefaultKernel, MIRAGE_AT_FDCWD};

pub(crate) unsafe fn mirage_openat_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    dirfd: i32,
    path: *const u8,
    flags: u32,
    mode: u32,
) -> isize {
    if path.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_OPENAT,
        [dirfd as u64, path as u64, flags as u64, mode as u64, 0, 0],
    )
}

pub(crate) unsafe fn mirage_open_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    path: *const u8,
    flags: u32,
    mode: u32,
) -> isize {
    mirage_openat_with_kernel(kernel, caller, MIRAGE_AT_FDCWD, path, flags, mode)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn openat(dirfd: i32, path: *const u8, flags: u32, mode: u32) -> isize {
    if path.is_null() {
        return errno::return_or_errno(-(MIRAGE_EFAULT as isize));
    }
    errno::return_or_errno(raw_syscall_runtime(
        syscall::MIRAGE_SYSCALL_OPENAT,
        [dirfd as u64, path as u64, flags as u64, mode as u64, 0, 0],
    ))
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn open(path: *const u8, flags: u32, mode: u32) -> isize {
    openat(MIRAGE_AT_FDCWD, path, flags, mode)
}
