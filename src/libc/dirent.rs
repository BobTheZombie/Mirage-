//! Directory entry C ABI wrappers.

use crate::kernel::fs::CDirEntry;
use crate::kernel::syscall::{self, MIRAGE_EFAULT};

use super::{raw_syscall_default, DefaultKernel};

#[no_mangle]
pub unsafe extern "C" fn mirage_getdents64(
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

#[no_mangle]
pub unsafe extern "C" fn getdents64(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    entries: *mut CDirEntry,
    count: usize,
) -> isize {
    mirage_getdents64(kernel, caller, fd, entries, count)
}
