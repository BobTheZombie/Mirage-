//! Filesystem metadata and mutation C ABI wrappers.

use crate::kernel::fs::CStat;
use crate::kernel::syscall::{self, MIRAGE_EFAULT};

use super::{raw_syscall_default, DefaultKernel, MIRAGE_AT_FDCWD};

#[no_mangle]
pub unsafe extern "C" fn mirage_statx(
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

#[no_mangle]
pub unsafe extern "C" fn mirage_newfstatat(
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

#[no_mangle]
pub unsafe extern "C" fn mirage_stat(
    kernel: *mut DefaultKernel,
    caller: u64,
    path: *const u8,
    out: *mut CStat,
) -> isize {
    mirage_newfstatat(kernel, caller, MIRAGE_AT_FDCWD, path, out, 0)
}

#[no_mangle]
pub unsafe extern "C" fn mirage_fstat(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    out: *mut CStat,
) -> isize {
    mirage_newfstatat(kernel, caller, fd, core::ptr::null(), out, 0)
}

#[no_mangle]
pub unsafe extern "C" fn mirage_mkdirat(
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

#[no_mangle]
pub unsafe extern "C" fn mirage_mkdir(
    kernel: *mut DefaultKernel,
    caller: u64,
    path: *const u8,
    mode: u32,
) -> isize {
    mirage_mkdirat(kernel, caller, MIRAGE_AT_FDCWD, path, mode)
}

#[no_mangle]
pub unsafe extern "C" fn mirage_unlinkat(
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

#[no_mangle]
pub unsafe extern "C" fn mirage_unlink(
    kernel: *mut DefaultKernel,
    caller: u64,
    path: *const u8,
) -> isize {
    mirage_unlinkat(kernel, caller, MIRAGE_AT_FDCWD, path, 0)
}

#[no_mangle]
pub unsafe extern "C" fn mirage_renameat2(
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

#[no_mangle]
pub unsafe extern "C" fn mirage_renameat(
    kernel: *mut DefaultKernel,
    caller: u64,
    old_dirfd: i32,
    old_path: *const u8,
    new_dirfd: i32,
    new_path: *const u8,
) -> isize {
    mirage_renameat2(kernel, caller, old_dirfd, old_path, new_dirfd, new_path, 0)
}

#[no_mangle]
pub unsafe extern "C" fn mirage_rename(
    kernel: *mut DefaultKernel,
    caller: u64,
    old_path: *const u8,
    new_path: *const u8,
) -> isize {
    mirage_renameat2(
        kernel,
        caller,
        MIRAGE_AT_FDCWD,
        old_path,
        MIRAGE_AT_FDCWD,
        new_path,
        0,
    )
}

#[no_mangle]
pub unsafe extern "C" fn mirage_fsync(kernel: *mut DefaultKernel, caller: u64, fd: i32) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_FSYNC,
        [fd as u64, 0, 0, 0, 0, 0],
    )
}

#[no_mangle]
pub unsafe extern "C" fn mirage_ftruncate(
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

#[no_mangle]
pub unsafe extern "C" fn stat(
    kernel: *mut DefaultKernel,
    caller: u64,
    path: *const u8,
    out: *mut CStat,
) -> isize {
    mirage_stat(kernel, caller, path, out)
}

#[no_mangle]
pub unsafe extern "C" fn fstat(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    out: *mut CStat,
) -> isize {
    mirage_fstat(kernel, caller, fd, out)
}

#[no_mangle]
pub unsafe extern "C" fn statx(
    kernel: *mut DefaultKernel,
    caller: u64,
    dirfd: i32,
    path: *const u8,
    flags: u32,
    mask: u32,
    out: *mut CStat,
) -> isize {
    mirage_statx(kernel, caller, dirfd, path, flags, mask, out)
}

#[no_mangle]
pub unsafe extern "C" fn mkdir(
    kernel: *mut DefaultKernel,
    caller: u64,
    path: *const u8,
    mode: u32,
) -> isize {
    mirage_mkdir(kernel, caller, path, mode)
}

#[no_mangle]
pub unsafe extern "C" fn mkdirat(
    kernel: *mut DefaultKernel,
    caller: u64,
    dirfd: i32,
    path: *const u8,
    mode: u32,
) -> isize {
    mirage_mkdirat(kernel, caller, dirfd, path, mode)
}

#[no_mangle]
pub unsafe extern "C" fn unlink(kernel: *mut DefaultKernel, caller: u64, path: *const u8) -> isize {
    mirage_unlink(kernel, caller, path)
}

#[no_mangle]
pub unsafe extern "C" fn unlinkat(
    kernel: *mut DefaultKernel,
    caller: u64,
    dirfd: i32,
    path: *const u8,
    flags: u32,
) -> isize {
    mirage_unlinkat(kernel, caller, dirfd, path, flags)
}

#[no_mangle]
pub unsafe extern "C" fn rename(
    kernel: *mut DefaultKernel,
    caller: u64,
    old_path: *const u8,
    new_path: *const u8,
) -> isize {
    mirage_rename(kernel, caller, old_path, new_path)
}

#[no_mangle]
pub unsafe extern "C" fn renameat(
    kernel: *mut DefaultKernel,
    caller: u64,
    old_dirfd: i32,
    old_path: *const u8,
    new_dirfd: i32,
    new_path: *const u8,
) -> isize {
    mirage_renameat(kernel, caller, old_dirfd, old_path, new_dirfd, new_path)
}

#[no_mangle]
pub unsafe extern "C" fn renameat2(
    kernel: *mut DefaultKernel,
    caller: u64,
    old_dirfd: i32,
    old_path: *const u8,
    new_dirfd: i32,
    new_path: *const u8,
    flags: u32,
) -> isize {
    mirage_renameat2(
        kernel, caller, old_dirfd, old_path, new_dirfd, new_path, flags,
    )
}

#[no_mangle]
pub unsafe extern "C" fn fsync(kernel: *mut DefaultKernel, caller: u64, fd: i32) -> isize {
    mirage_fsync(kernel, caller, fd)
}

#[no_mangle]
pub unsafe extern "C" fn ftruncate(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    size: u64,
) -> isize {
    mirage_ftruncate(kernel, caller, fd, size)
}
