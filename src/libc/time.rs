//! Time-related C ABI wrappers.

use crate::kernel::syscall::{self, MIRAGE_EFAULT};
use crate::kernel::MirageTimespec;

use super::{raw_syscall_default, DefaultKernel};

#[no_mangle]
pub unsafe extern "C" fn mirage_clock_gettime(
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

#[no_mangle]
pub unsafe extern "C" fn clock_gettime(
    kernel: *mut DefaultKernel,
    caller: u64,
    clock_id: i32,
    out: *mut MirageTimespec,
) -> isize {
    mirage_clock_gettime(kernel, caller, clock_id, out)
}
