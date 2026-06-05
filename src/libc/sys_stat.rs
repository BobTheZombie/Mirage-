//! Filesystem metadata and mutation C ABI wrappers.
#![allow(dead_code)]

use crate::kernel::fs::CStat;
use crate::kernel::syscall::{self, MIRAGE_EFAULT};

use super::{errno, raw_syscall_default, raw_syscall_runtime, DefaultKernel, MIRAGE_AT_FDCWD};

pub(crate) unsafe fn mirage_statx_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    dirfd: i32,
    path: *const u8,
    _flags: u32,
    _mask: u32,
    out: *mut CStat,
) -> isize {
    if path.is_null() || out.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_STATX,
        [dirfd as u64, path as u64, out as u64, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_newfstatat_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    dirfd: i32,
    path: *const u8,
    out: *mut CStat,
    flags: u32,
) -> isize {
    let path_arg = if path.is_null() { 0 } else { path as u64 };
    if out.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_NEWFSTATAT,
        [dirfd as u64, path_arg, out as u64, flags as u64, 0, 0],
    )
}

pub(crate) unsafe fn mirage_stat_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    path: *const u8,
    out: *mut CStat,
) -> isize {
    mirage_newfstatat_with_kernel(kernel, caller, MIRAGE_AT_FDCWD, path, out, 0)
}

pub(crate) unsafe fn mirage_fstat_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    out: *mut CStat,
) -> isize {
    mirage_newfstatat_with_kernel(kernel, caller, fd, core::ptr::null(), out, 0)
}

pub(crate) unsafe fn mirage_mkdirat_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    dirfd: i32,
    path: *const u8,
    mode: u32,
) -> isize {
    if path.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_MKDIRAT,
        [dirfd as u64, path as u64, mode as u64, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_mkdir_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    path: *const u8,
    mode: u32,
) -> isize {
    mirage_mkdirat_with_kernel(kernel, caller, MIRAGE_AT_FDCWD, path, mode)
}

pub(crate) unsafe fn mirage_unlinkat_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    dirfd: i32,
    path: *const u8,
    flags: u32,
) -> isize {
    if path.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_UNLINKAT,
        [dirfd as u64, path as u64, flags as u64, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_unlink_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    path: *const u8,
) -> isize {
    mirage_unlinkat_with_kernel(kernel, caller, MIRAGE_AT_FDCWD, path, 0)
}

pub(crate) unsafe fn mirage_renameat2_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    old_dirfd: i32,
    old_path: *const u8,
    new_dirfd: i32,
    new_path: *const u8,
    flags: u32,
) -> isize {
    if old_path.is_null() || new_path.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_RENAMEAT2,
        [
            old_dirfd as u64,
            old_path as u64,
            new_dirfd as u64,
            new_path as u64,
            flags as u64,
            0,
        ],
    )
}

pub(crate) unsafe fn mirage_renameat_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    old_dirfd: i32,
    old_path: *const u8,
    new_dirfd: i32,
    new_path: *const u8,
) -> isize {
    mirage_renameat2_with_kernel(kernel, caller, old_dirfd, old_path, new_dirfd, new_path, 0)
}

pub(crate) unsafe fn mirage_rename_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    old_path: *const u8,
    new_path: *const u8,
) -> isize {
    mirage_renameat2_with_kernel(
        kernel,
        caller,
        MIRAGE_AT_FDCWD,
        old_path,
        MIRAGE_AT_FDCWD,
        new_path,
        0,
    )
}

pub(crate) unsafe fn mirage_fsync_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_FSYNC,
        [fd as u64, 0, 0, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_ftruncate_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    size: u64,
) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_FTRUNCATE,
        [fd as u64, size, 0, 0, 0, 0],
    )
}

fn posix_syscall(number: u64, args: [u64; crate::kernel::syscall::SYSCALL_MAX_ARGS]) -> isize {
    errno::return_or_errno(raw_syscall_runtime(number, args))
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn stat(path: *const u8, out: *mut CStat) -> isize {
    newfstatat(MIRAGE_AT_FDCWD, path, out, 0)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fstat(fd: i32, out: *mut CStat) -> isize {
    newfstatat(fd, core::ptr::null(), out, 0)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn statx(
    dirfd: i32,
    path: *const u8,
    _flags: u32,
    _mask: u32,
    out: *mut CStat,
) -> isize {
    if path.is_null() || out.is_null() {
        return errno::return_or_errno(-(MIRAGE_EFAULT as isize));
    }
    posix_syscall(
        syscall::MIRAGE_SYSCALL_STATX,
        [dirfd as u64, path as u64, out as u64, 0, 0, 0],
    )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn newfstatat(
    dirfd: i32,
    path: *const u8,
    out: *mut CStat,
    flags: u32,
) -> isize {
    let path_arg = if path.is_null() { 0 } else { path as u64 };
    if out.is_null() {
        return errno::return_or_errno(-(MIRAGE_EFAULT as isize));
    }
    posix_syscall(
        syscall::MIRAGE_SYSCALL_NEWFSTATAT,
        [dirfd as u64, path_arg, out as u64, flags as u64, 0, 0],
    )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn mkdir(path: *const u8, mode: u32) -> isize {
    mkdirat(MIRAGE_AT_FDCWD, path, mode)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn mkdirat(dirfd: i32, path: *const u8, mode: u32) -> isize {
    if path.is_null() {
        return errno::return_or_errno(-(MIRAGE_EFAULT as isize));
    }
    posix_syscall(
        syscall::MIRAGE_SYSCALL_MKDIRAT,
        [dirfd as u64, path as u64, mode as u64, 0, 0, 0],
    )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn unlink(path: *const u8) -> isize {
    unlinkat(MIRAGE_AT_FDCWD, path, 0)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn unlinkat(dirfd: i32, path: *const u8, flags: u32) -> isize {
    if path.is_null() {
        return errno::return_or_errno(-(MIRAGE_EFAULT as isize));
    }
    posix_syscall(
        syscall::MIRAGE_SYSCALL_UNLINKAT,
        [dirfd as u64, path as u64, flags as u64, 0, 0, 0],
    )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn rename(old_path: *const u8, new_path: *const u8) -> isize {
    renameat2(MIRAGE_AT_FDCWD, old_path, MIRAGE_AT_FDCWD, new_path, 0)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn renameat(
    old_dirfd: i32,
    old_path: *const u8,
    new_dirfd: i32,
    new_path: *const u8,
) -> isize {
    renameat2(old_dirfd, old_path, new_dirfd, new_path, 0)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn renameat2(
    old_dirfd: i32,
    old_path: *const u8,
    new_dirfd: i32,
    new_path: *const u8,
    flags: u32,
) -> isize {
    if old_path.is_null() || new_path.is_null() {
        return errno::return_or_errno(-(MIRAGE_EFAULT as isize));
    }
    posix_syscall(
        syscall::MIRAGE_SYSCALL_RENAMEAT2,
        [
            old_dirfd as u64,
            old_path as u64,
            new_dirfd as u64,
            new_path as u64,
            flags as u64,
            0,
        ],
    )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fsync(fd: i32) -> isize {
    posix_syscall(syscall::MIRAGE_SYSCALL_FSYNC, [fd as u64, 0, 0, 0, 0, 0])
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn ftruncate(fd: i32, size: u64) -> isize {
    posix_syscall(
        syscall::MIRAGE_SYSCALL_FTRUNCATE,
        [fd as u64, size, 0, 0, 0, 0],
    )
}
