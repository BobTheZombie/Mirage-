//! Open and descriptor flag C ABI exports.

use crate::kernel::syscall::{self, MIRAGE_EFAULT};

use super::{raw_syscall_default, DefaultKernel, MIRAGE_AT_FDCWD};

#[no_mangle]
pub unsafe extern "C" fn mirage_openat(
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

/// Mirage `open(2)`-style wrapper using `AT_FDCWD`.
#[no_mangle]
pub unsafe extern "C" fn mirage_open(
    kernel: *mut DefaultKernel,
    caller: u64,
    path: *const u8,
    flags: u32,
    mode: u32,
) -> isize {
    mirage_openat(kernel, caller, MIRAGE_AT_FDCWD, path, flags, mode)
}

#[no_mangle]
pub unsafe extern "C" fn openat(
    kernel: *mut DefaultKernel,
    caller: u64,
    dirfd: i32,
    path: *const u8,
    flags: u32,
    mode: u32,
) -> isize {
    mirage_openat(kernel, caller, dirfd, path, flags, mode)
}

#[no_mangle]
pub unsafe extern "C" fn open(
    kernel: *mut DefaultKernel,
    caller: u64,
    path: *const u8,
    flags: u32,
    mode: u32,
) -> isize {
    mirage_open(kernel, caller, path, flags, mode)
}
