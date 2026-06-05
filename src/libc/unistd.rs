//! POSIX process and file descriptor C ABI wrappers.
#![allow(dead_code)]

use crate::kernel::device::{DeviceId, MirageDeviceDescriptor};
use crate::kernel::process::ProcessId;
use crate::kernel::syscall::{self, MIRAGE_EFAULT, SYSCALL_MAX_ARGS};
use crate::kernel::Kernel;

use super::{
    device_info, device_read, device_write, enumerate_devices, errno, raw_syscall_default,
    raw_syscall_runtime, DefaultKernel,
};

pub(crate) unsafe fn mirage_close_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_CLOSE,
        [fd as u64, 0, 0, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_read_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    buffer: *mut u8,
    len: usize,
) -> isize {
    if len > 0 && buffer.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_READ,
        [fd as u64, buffer as u64, len as u64, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_write_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    data: *const u8,
    len: usize,
) -> isize {
    if len > 0 && data.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_WRITE,
        [fd as u64, data as u64, len as u64, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_lseek_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    offset: i64,
    whence: u32,
) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_LSEEK,
        [fd as u64, offset as u64, whence as u64, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_getpid_with_kernel(kernel: *mut DefaultKernel, caller: u64) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETPID,
        [0; SYSCALL_MAX_ARGS],
    )
}

pub(crate) unsafe fn mirage_getppid_with_kernel(kernel: *mut DefaultKernel, caller: u64) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETPPID,
        [0; SYSCALL_MAX_ARGS],
    )
}

pub(crate) unsafe fn mirage_getuid_with_kernel(kernel: *mut DefaultKernel, caller: u64) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETUID,
        [0; SYSCALL_MAX_ARGS],
    )
}

pub(crate) unsafe fn mirage_geteuid_with_kernel(kernel: *mut DefaultKernel, caller: u64) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETEUID,
        [0; SYSCALL_MAX_ARGS],
    )
}

pub(crate) unsafe fn mirage_getgid_with_kernel(kernel: *mut DefaultKernel, caller: u64) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETGID,
        [0; SYSCALL_MAX_ARGS],
    )
}

pub(crate) unsafe fn mirage_getegid_with_kernel(kernel: *mut DefaultKernel, caller: u64) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETGID,
        [0; SYSCALL_MAX_ARGS],
    )
}

pub(crate) unsafe fn mirage_setuid_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    uid: u32,
) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_SETUID,
        [uid as u64, 0, 0, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_setgid_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    gid: u32,
) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_SETGID,
        [gid as u64, 0, 0, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_getgroups_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    size: usize,
    groups: *mut u32,
) -> isize {
    if size > 0 && groups.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETGROUPS,
        [size as u64, groups as u64, 0, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_exit_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    status: i32,
) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_EXIT,
        [status as u64, 0, 0, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_kill_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    pid: u64,
    signal: i32,
) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_KILL,
        [pid, signal as u64, 0, 0, 0, 0],
    )
}

pub(crate) unsafe fn mirage_clone_with_kernel(
    kernel: *mut DefaultKernel,
    caller: u64,
    entry_point: u64,
    priority: u64,
) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_CLONE,
        [entry_point, priority, 0, 0, 0, 0],
    )
}

fn posix_syscall(number: u64, args: [u64; SYSCALL_MAX_ARGS]) -> isize {
    errno::return_or_errno(raw_syscall_runtime(number, args))
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn close(fd: i32) -> isize {
    posix_syscall(syscall::MIRAGE_SYSCALL_CLOSE, [fd as u64, 0, 0, 0, 0, 0])
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn read(fd: i32, buffer: *mut u8, len: usize) -> isize {
    if len > 0 && buffer.is_null() {
        return errno::return_or_errno(-(MIRAGE_EFAULT as isize));
    }
    posix_syscall(
        syscall::MIRAGE_SYSCALL_READ,
        [fd as u64, buffer as u64, len as u64, 0, 0, 0],
    )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn write(fd: i32, data: *const u8, len: usize) -> isize {
    if len > 0 && data.is_null() {
        return errno::return_or_errno(-(MIRAGE_EFAULT as isize));
    }
    posix_syscall(
        syscall::MIRAGE_SYSCALL_WRITE,
        [fd as u64, data as u64, len as u64, 0, 0, 0],
    )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn lseek(fd: i32, offset: i64, whence: u32) -> isize {
    posix_syscall(
        syscall::MIRAGE_SYSCALL_LSEEK,
        [fd as u64, offset as u64, whence as u64, 0, 0, 0],
    )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getpid() -> isize {
    posix_syscall(syscall::MIRAGE_SYSCALL_GETPID, [0; SYSCALL_MAX_ARGS])
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getppid() -> isize {
    posix_syscall(syscall::MIRAGE_SYSCALL_GETPPID, [0; SYSCALL_MAX_ARGS])
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getuid() -> isize {
    posix_syscall(syscall::MIRAGE_SYSCALL_GETUID, [0; SYSCALL_MAX_ARGS])
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn geteuid() -> isize {
    posix_syscall(syscall::MIRAGE_SYSCALL_GETEUID, [0; SYSCALL_MAX_ARGS])
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getgid() -> isize {
    posix_syscall(syscall::MIRAGE_SYSCALL_GETGID, [0; SYSCALL_MAX_ARGS])
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getegid() -> isize {
    posix_syscall(syscall::MIRAGE_SYSCALL_GETGID, [0; SYSCALL_MAX_ARGS])
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn setuid(uid: u32) -> isize {
    posix_syscall(syscall::MIRAGE_SYSCALL_SETUID, [uid as u64, 0, 0, 0, 0, 0])
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn setgid(gid: u32) -> isize {
    posix_syscall(syscall::MIRAGE_SYSCALL_SETGID, [gid as u64, 0, 0, 0, 0, 0])
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getgroups(size: usize, groups: *mut u32) -> isize {
    if size > 0 && groups.is_null() {
        return errno::return_or_errno(-(MIRAGE_EFAULT as isize));
    }
    posix_syscall(
        syscall::MIRAGE_SYSCALL_GETGROUPS,
        [size as u64, groups as u64, 0, 0, 0, 0],
    )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _exit(status: i32) -> isize {
    posix_syscall(syscall::MIRAGE_SYSCALL_EXIT, [status as u64, 0, 0, 0, 0, 0])
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn kill(pid: u64, signal: i32) -> isize {
    posix_syscall(
        syscall::MIRAGE_SYSCALL_KILL,
        [pid, signal as u64, 0, 0, 0, 0],
    )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn clone(entry_point: u64, priority: u64) -> isize {
    posix_syscall(
        syscall::MIRAGE_SYSCALL_CLONE,
        [entry_point, priority, 0, 0, 0, 0],
    )
}

pub unsafe extern "C" fn mirage_device_enumerate(
    kernel: *mut Kernel<{ crate::kernel::MAX_PROCESSES }, { crate::kernel::MESSAGE_DEPTH }>,
    caller: u64,
    out: *mut MirageDeviceDescriptor,
    capacity: usize,
) -> isize {
    if kernel.is_null() || (capacity > 0 && out.is_null()) {
        return -(MIRAGE_EFAULT as isize);
    }
    let out_slice = if capacity == 0 {
        &mut []
    } else {
        core::slice::from_raw_parts_mut(out, capacity)
    };
    match enumerate_devices(&mut *kernel, ProcessId::new(caller), None, out_slice) {
        Ok(count) => count as isize,
        Err(err) => -(errno::libc_errno(err) as isize),
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn mirage_device_info(
    kernel: *mut Kernel<{ crate::kernel::MAX_PROCESSES }, { crate::kernel::MESSAGE_DEPTH }>,
    caller: u64,
    id: u16,
    out: *mut MirageDeviceDescriptor,
) -> isize {
    if kernel.is_null() || out.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    match device_info(
        &mut *kernel,
        ProcessId::new(caller),
        None,
        DeviceId::new(id),
        &mut *out,
    ) {
        Ok(()) => 0,
        Err(err) => -(errno::libc_errno(err) as isize),
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn mirage_device_read(
    kernel: *mut Kernel<{ crate::kernel::MAX_PROCESSES }, { crate::kernel::MESSAGE_DEPTH }>,
    caller: u64,
    id: u16,
    buffer: *mut u8,
    len: usize,
) -> isize {
    if kernel.is_null() || (len > 0 && buffer.is_null()) {
        return -(MIRAGE_EFAULT as isize);
    }
    let buffer = if len == 0 {
        &mut []
    } else {
        core::slice::from_raw_parts_mut(buffer, len)
    };
    match device_read(
        &mut *kernel,
        ProcessId::new(caller),
        None,
        DeviceId::new(id),
        buffer,
    ) {
        Ok(read) => read as isize,
        Err(err) => -(errno::libc_errno(err) as isize),
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn mirage_device_write(
    kernel: *mut Kernel<{ crate::kernel::MAX_PROCESSES }, { crate::kernel::MESSAGE_DEPTH }>,
    caller: u64,
    id: u16,
    data: *const u8,
    len: usize,
) -> isize {
    if kernel.is_null() || (len > 0 && data.is_null()) {
        return -(MIRAGE_EFAULT as isize);
    }
    let data = if len == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(data, len)
    };
    match device_write(
        &mut *kernel,
        ProcessId::new(caller),
        None,
        DeviceId::new(id),
        data,
    ) {
        Ok(written) => written as isize,
        Err(err) => -(errno::libc_errno(err) as isize),
    }
}
