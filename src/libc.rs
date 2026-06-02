//! User-facing syscall wrappers.
//!
//! These helpers keep libc-style services on the public syscall ABI instead of
//! reaching into kernel internals directly. Synchronous wrappers still call the
//! kernel entry point directly, while [`queue_syscall_trap`] models the register
//! handoff a running thread would perform with the CPU syscall instruction.

use core::ffi::c_void;

use crate::kernel::device::{DeviceId, MirageDeviceDescriptor};
use crate::kernel::fs::{errno_from_vfs, CDirEntry, CStat};
use crate::kernel::ipc::Message;
use crate::kernel::memory::MemoryProtection;
use crate::kernel::process::{ProcessId, ProcessPriority};
use crate::kernel::syscall::{
    SyscallContext, SyscallNumber, MIRAGE_EACCES, MIRAGE_EAGAIN, MIRAGE_EFAULT, MIRAGE_EINVAL,
    MIRAGE_EIO, MIRAGE_ENOBUFS, MIRAGE_ENOMEM, MIRAGE_ENOSYS, MIRAGE_ESRCH, MIRAGE_SYSCALL_CLOSE,
    MIRAGE_SYSCALL_FSYNC, MIRAGE_SYSCALL_FTRUNCATE, MIRAGE_SYSCALL_GETDENTS64,
    MIRAGE_SYSCALL_LSEEK, MIRAGE_SYSCALL_MKDIRAT, MIRAGE_SYSCALL_NEWFSTATAT, MIRAGE_SYSCALL_OPENAT,
    MIRAGE_SYSCALL_READ, MIRAGE_SYSCALL_RENAMEAT2, MIRAGE_SYSCALL_STATX, MIRAGE_SYSCALL_UNLINKAT,
    MIRAGE_SYSCALL_WRITE, SYSCALL_MAX_ARGS,
};
use crate::kernel::thread::ThreadId;
use crate::kernel::{Kernel, KernelError, KernelResult};
use crate::subkernel::{IsolationError, SecurityClass};

type DefaultKernel = Kernel<{ crate::kernel::MAX_PROCESSES }, { crate::kernel::MESSAGE_DEPTH }>;

const MIRAGE_AT_FDCWD: i32 = -100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnCredentialProfile {
    User = 0,
    System = 1,
}

pub fn queue_syscall_trap<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    thread: ThreadId,
    number: SyscallNumber,
    args: [u64; SYSCALL_MAX_ARGS],
) -> KernelResult<()> {
    kernel.queue_thread_syscall(thread, number.raw(), args)
}

pub fn getpid<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
) -> KernelResult<ProcessId> {
    syscall(
        kernel,
        caller,
        thread,
        SyscallNumber::GetPid,
        [0; SYSCALL_MAX_ARGS],
    )
    .map(ProcessId::new)
}

pub fn spawn<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    entry_point: u64,
    priority: ProcessPriority,
    credential_profile: SpawnCredentialProfile,
) -> KernelResult<ProcessId> {
    let args = [
        entry_point,
        encode_priority(priority),
        credential_profile as u64,
        0,
        0,
        0,
    ];
    syscall(kernel, caller, thread, SyscallNumber::Spawn, args).map(ProcessId::new)
}

pub fn send_ipc<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    receiver: ProcessId,
    data: &[u8],
    security_class: SecurityClass,
) -> KernelResult<usize> {
    let args = [
        receiver.raw(),
        data.as_ptr() as u64,
        data.len() as u64,
        encode_security_class(security_class),
        0,
        0,
    ];
    syscall(kernel, caller, thread, SyscallNumber::SendIpc, args).map(|written| written as usize)
}

pub fn receive_ipc<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    out: &mut Message,
) -> KernelResult<usize> {
    receive_ipc_or_block(kernel, caller, thread, out).map(|read| read.unwrap_or(0))
}

pub fn receive_ipc_or_block<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    out: &mut Message,
) -> KernelResult<Option<usize>> {
    let args = [out as *mut Message as u64, 0, 0, 0, 0, 0];
    syscall(
        kernel,
        caller,
        thread,
        SyscallNumber::ReceiveOrBlockIpc,
        args,
    )
    .map(|received| {
        if received == 0 {
            None
        } else {
            Some(out.payload.length)
        }
    })
}

pub fn block_for_ipc<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
) -> KernelResult<()> {
    syscall(
        kernel,
        caller,
        thread,
        SyscallNumber::BlockForIpc,
        [0; SYSCALL_MAX_ARGS],
    )
    .map(|_| ())
}

pub fn enumerate_devices<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    out: &mut [MirageDeviceDescriptor],
) -> KernelResult<usize> {
    let args = [out.as_mut_ptr() as u64, out.len() as u64, 0, 0, 0, 0];
    syscall(
        kernel,
        caller,
        thread,
        SyscallNumber::EnumerateDevices,
        args,
    )
    .map(|count| count as usize)
}

pub fn device_info<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    id: DeviceId,
    out: &mut MirageDeviceDescriptor,
) -> KernelResult<()> {
    let args = [
        id.raw() as u64,
        out as *mut MirageDeviceDescriptor as u64,
        0,
        0,
        0,
        0,
    ];
    syscall(kernel, caller, thread, SyscallNumber::DeviceInfo, args).map(|_| ())
}

pub fn device_read<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    id: DeviceId,
    buffer: &mut [u8],
) -> KernelResult<usize> {
    let args = [
        id.raw() as u64,
        buffer.as_mut_ptr() as u64,
        buffer.len() as u64,
        0,
        0,
        0,
    ];
    syscall(kernel, caller, thread, SyscallNumber::DeviceRead, args).map(|read| read as usize)
}

pub fn device_write<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    id: DeviceId,
    data: &[u8],
) -> KernelResult<usize> {
    let args = [
        id.raw() as u64,
        data.as_ptr() as u64,
        data.len() as u64,
        0,
        0,
        0,
    ];
    syscall(kernel, caller, thread, SyscallNumber::DeviceWrite, args)
        .map(|written| written as usize)
}

pub fn mmap<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    length: usize,
    protection: MemoryProtection,
) -> KernelResult<*mut c_void> {
    let args = [length as u64, protection.bits() as u64, 0, 0, 0, 0];
    syscall(kernel, caller, thread, SyscallNumber::Mmap, args).map(|ptr| ptr as *mut c_void)
}

pub fn munmap<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    ptr: *mut c_void,
    length: usize,
) -> KernelResult<()> {
    let args = [ptr as u64, length as u64, 0, 0, 0, 0];
    syscall(kernel, caller, thread, SyscallNumber::Munmap, args).map(|_| ())
}

pub fn malloc<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    size: usize,
) -> KernelResult<*mut c_void> {
    let args = [size as u64, 0, 0, 0, 0, 0];
    syscall(kernel, caller, thread, SyscallNumber::Malloc, args).map(|ptr| ptr as *mut c_void)
}

pub fn free<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    ptr: *mut c_void,
) -> KernelResult<()> {
    let args = [ptr as u64, 0, 0, 0, 0, 0];
    syscall(kernel, caller, thread, SyscallNumber::Free, args).map(|_| ())
}

fn raw_syscall_default(
    kernel: *mut DefaultKernel,
    caller: u64,
    number: u64,
    args: [u64; SYSCALL_MAX_ARGS],
) -> isize {
    if kernel.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }
    let result = unsafe {
        (&mut *kernel).handle_syscall(
            number,
            SyscallContext::new(ProcessId::new(caller), None, args),
        )
    };
    match result {
        Ok(value) => value as isize,
        Err(error) => -(libc_errno(error) as isize),
    }
}

/// Mirage `openat(2)`-style wrapper over `MIRAGE_SYSCALL_OPENAT`.
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
        MIRAGE_SYSCALL_OPENAT,
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
pub unsafe extern "C" fn mirage_close(kernel: *mut DefaultKernel, caller: u64, fd: i32) -> isize {
    raw_syscall_default(
        kernel,
        caller,
        MIRAGE_SYSCALL_CLOSE,
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
        MIRAGE_SYSCALL_READ,
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
        MIRAGE_SYSCALL_WRITE,
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
        MIRAGE_SYSCALL_LSEEK,
        [fd as u64, offset as u64, whence as u64, 0, 0, 0],
    )
}

/// Mirage `statx(2)`-compatible wrapper writing a C-compatible stat payload.
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
        MIRAGE_SYSCALL_STATX,
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
        MIRAGE_SYSCALL_NEWFSTATAT,
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
        MIRAGE_SYSCALL_MKDIRAT,
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
        MIRAGE_SYSCALL_UNLINKAT,
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
        MIRAGE_SYSCALL_RENAMEAT2,
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
        MIRAGE_SYSCALL_FSYNC,
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
        MIRAGE_SYSCALL_FTRUNCATE,
        [fd as u64, size, 0, 0, 0, 0],
    )
}

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
        MIRAGE_SYSCALL_GETDENTS64,
        [fd as u64, entries as u64, count as u64, 0, 0, 0],
    )
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

#[no_mangle]
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
        Err(err) => -(libc_errno(err) as isize),
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
        Err(err) => -(libc_errno(err) as isize),
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
        Err(err) => -(libc_errno(err) as isize),
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
        Err(err) => -(libc_errno(err) as isize),
    }
}

fn libc_errno(error: KernelError) -> i32 {
    match error {
        KernelError::ProcessTableFull
        | KernelError::SchedulerFull
        | KernelError::ThreadTableFull
        | KernelError::AllocationFailed
        | KernelError::FileTableFull => MIRAGE_ENOMEM,
        KernelError::UnknownProcess | KernelError::UnknownThread => MIRAGE_ESRCH,
        KernelError::MessageQueueFull => MIRAGE_ENOBUFS,
        KernelError::MessageQueueEmpty => MIRAGE_EAGAIN,
        KernelError::SecurityViolation(IsolationError::UnknownTask)
        | KernelError::IsolationFault(IsolationError::UnknownTask) => MIRAGE_ESRCH,
        KernelError::SecurityViolation(
            IsolationError::CapabilityMissing | IsolationError::PolicyViolation,
        )
        | KernelError::IsolationFault(
            IsolationError::CapabilityMissing | IsolationError::PolicyViolation,
        ) => MIRAGE_EACCES,
        KernelError::DeviceNotFound => MIRAGE_ESRCH,
        KernelError::DeviceFault(_) => MIRAGE_EIO,
        KernelError::InvalidSyscall => MIRAGE_ENOSYS,
        KernelError::InvalidArgument => MIRAGE_EINVAL,
        KernelError::InvalidPointer => MIRAGE_EFAULT,
        KernelError::Filesystem(error) => libc_vfs_errno(error),
    }
}

fn libc_vfs_errno(error: crate::kernel::fs::VfsError) -> i32 {
    errno_from_vfs(error)
}

fn syscall<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    number: SyscallNumber,
    args: [u64; SYSCALL_MAX_ARGS],
) -> KernelResult<u64> {
    kernel.handle_syscall(number.raw(), SyscallContext::new(caller, thread, args))
}

fn encode_priority(priority: ProcessPriority) -> u64 {
    match priority {
        ProcessPriority::Critical => 0,
        ProcessPriority::High => 1,
        ProcessPriority::Normal => 2,
        ProcessPriority::Low => 3,
    }
}

fn encode_security_class(class: SecurityClass) -> u64 {
    match class {
        SecurityClass::Public => 0,
        SecurityClass::Internal => 1,
        SecurityClass::Confidential => 2,
        SecurityClass::System => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libc_errno_maps_security_and_queue_failures() {
        assert_eq!(
            libc_errno(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            )),
            MIRAGE_EACCES
        );
        assert_eq!(
            libc_errno(KernelError::SecurityViolation(IsolationError::UnknownTask)),
            MIRAGE_ESRCH
        );
        assert_eq!(libc_errno(KernelError::MessageQueueFull), MIRAGE_ENOBUFS);
    }
}
