//! POSIX process and file descriptor C ABI wrappers.

use crate::kernel::device::{DeviceId, MirageDeviceDescriptor};
use crate::kernel::process::ProcessId;
use crate::kernel::syscall::{self, MIRAGE_EFAULT, SYSCALL_MAX_ARGS};
use crate::kernel::Kernel;

use super::{
    device_info, device_read, device_write, enumerate_devices, errno, raw_syscall_default,
    DefaultKernel,
};

#[no_mangle]
pub unsafe extern "C" fn mirage_close(kernel: *mut DefaultKernel, caller: u64, fd: i32) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_CLOSE,
        [fd as u64, 0, 0, 0, 0, 0],
    )
}

#[no_mangle]
pub unsafe extern "C" fn mirage_read(
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

#[no_mangle]
pub unsafe extern "C" fn mirage_write(
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

#[no_mangle]
pub unsafe extern "C" fn mirage_lseek(
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

/// Mirage `statx(2)`-compatible wrapper writing a C-compatible stat payload.
#[no_mangle]
pub unsafe extern "C" fn mirage_getpid(kernel: *mut DefaultKernel, caller: u64) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETPID,
        [0; SYSCALL_MAX_ARGS],
    )
}

#[no_mangle]
pub unsafe extern "C" fn mirage_getppid(kernel: *mut DefaultKernel, caller: u64) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETPPID,
        [0; SYSCALL_MAX_ARGS],
    )
}

#[no_mangle]
pub unsafe extern "C" fn mirage_getuid(kernel: *mut DefaultKernel, caller: u64) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETUID,
        [0; SYSCALL_MAX_ARGS],
    )
}

#[no_mangle]
pub unsafe extern "C" fn mirage_geteuid(kernel: *mut DefaultKernel, caller: u64) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETEUID,
        [0; SYSCALL_MAX_ARGS],
    )
}

#[no_mangle]
pub unsafe extern "C" fn mirage_getgid(kernel: *mut DefaultKernel, caller: u64) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETGID,
        [0; SYSCALL_MAX_ARGS],
    )
}

#[no_mangle]
pub unsafe extern "C" fn mirage_getegid(kernel: *mut DefaultKernel, caller: u64) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_GETGID,
        [0; SYSCALL_MAX_ARGS],
    )
}

#[no_mangle]
pub unsafe extern "C" fn mirage_setuid(kernel: *mut DefaultKernel, caller: u64, uid: u32) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_SETUID,
        [uid as u64, 0, 0, 0, 0, 0],
    )
}

#[no_mangle]
pub unsafe extern "C" fn mirage_setgid(kernel: *mut DefaultKernel, caller: u64, gid: u32) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        syscall::MIRAGE_SYSCALL_SETGID,
        [gid as u64, 0, 0, 0, 0, 0],
    )
}

#[no_mangle]
pub unsafe extern "C" fn mirage_getgroups(
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

#[no_mangle]
pub unsafe extern "C" fn mirage_exit(
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

#[no_mangle]
pub unsafe extern "C" fn mirage_kill(
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

#[no_mangle]
pub unsafe extern "C" fn mirage_clone(
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

#[no_mangle]
pub unsafe extern "C" fn close(kernel: *mut DefaultKernel, caller: u64, fd: i32) -> isize {
    mirage_close(kernel, caller, fd)
}

#[no_mangle]
pub unsafe extern "C" fn read(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    buffer: *mut u8,
    len: usize,
) -> isize {
    mirage_read(kernel, caller, fd, buffer, len)
}

#[no_mangle]
pub unsafe extern "C" fn write(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    data: *const u8,
    len: usize,
) -> isize {
    mirage_write(kernel, caller, fd, data, len)
}

#[no_mangle]
pub unsafe extern "C" fn lseek(
    kernel: *mut DefaultKernel,
    caller: u64,
    fd: i32,
    offset: i64,
    whence: u32,
) -> isize {
    mirage_lseek(kernel, caller, fd, offset, whence)
}

#[no_mangle]
pub unsafe extern "C" fn getpid(kernel: *mut DefaultKernel, caller: u64) -> isize {
    mirage_getpid(kernel, caller)
}

#[no_mangle]
pub unsafe extern "C" fn getppid(kernel: *mut DefaultKernel, caller: u64) -> isize {
    mirage_getppid(kernel, caller)
}

#[no_mangle]
pub unsafe extern "C" fn getuid(kernel: *mut DefaultKernel, caller: u64) -> isize {
    mirage_getuid(kernel, caller)
}

#[no_mangle]
pub unsafe extern "C" fn geteuid(kernel: *mut DefaultKernel, caller: u64) -> isize {
    mirage_geteuid(kernel, caller)
}

#[no_mangle]
pub unsafe extern "C" fn getgid(kernel: *mut DefaultKernel, caller: u64) -> isize {
    mirage_getgid(kernel, caller)
}

#[no_mangle]
pub unsafe extern "C" fn getegid(kernel: *mut DefaultKernel, caller: u64) -> isize {
    mirage_getegid(kernel, caller)
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

#[no_mangle]
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

#[no_mangle]
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

#[no_mangle]
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
