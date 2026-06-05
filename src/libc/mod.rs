//! User-facing syscall wrappers and C ABI runtime exports.
//!
//! Public C ABI symbols live in focused submodules. Rust-facing syscall helpers
//! remain available from this module through re-exports and direct definitions.

use core::ffi::c_void;

use crate::kernel::device::{DeviceId, MirageDeviceDescriptor};
use crate::kernel::ipc::Message;
use crate::kernel::memory::MemoryProtection;
use crate::kernel::process::{ProcessId, ProcessPriority};
use crate::kernel::syscall::{SyscallContext, SyscallNumber, SYSCALL_MAX_ARGS};
use crate::kernel::thread::ThreadId;
use crate::kernel::{Kernel, KernelResult, MirageTimespec};
use crate::subkernel::SecurityClass;

pub mod dirent;
pub mod errno;
pub mod fcntl;
pub mod pthread;
pub mod socket;
pub mod stdlib;
pub mod string;
pub mod sys_stat;
pub mod time;
pub mod unistd;

pub use dirent::getdents64;
pub use fcntl::{open, openat};
pub use sys_stat::{
    fstat, fsync, ftruncate, mkdir, mkdirat, newfstatat, rename, renameat, renameat2, stat, statx,
    unlink, unlinkat,
};
pub use time::clock_gettime;
pub use unistd::{
    _exit, clone, close, getegid, geteuid, getgid, getgroups, getpid, getppid, getuid, kill, lseek,
    mirage_device_enumerate, mirage_device_info, mirage_device_read, mirage_device_write, read,
    setgid, setuid, write,
};

pub(super) type DefaultKernel =
    Kernel<{ crate::kernel::MAX_PROCESSES }, { crate::kernel::MESSAGE_DEPTH }>;

pub(super) const MIRAGE_AT_FDCWD: i32 = -100;

#[derive(Clone, Copy)]
pub struct MirageRuntimeSyscallContext {
    pub kernel: *mut DefaultKernel,
    pub caller: ProcessId,
    pub thread: Option<ThreadId>,
}

static mut RUNTIME_SYSCALL_CONTEXT: MirageRuntimeSyscallContext = MirageRuntimeSyscallContext {
    kernel: core::ptr::null_mut(),
    caller: ProcessId::new(0),
    thread: None,
};

/// Installs the kernel/process/thread context used by POSIX C ABI shims.
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn mirage_set_runtime_syscall_context(
    kernel: *mut DefaultKernel,
    caller: u64,
    thread: u64,
) {
    RUNTIME_SYSCALL_CONTEXT = MirageRuntimeSyscallContext {
        kernel,
        caller: ProcessId::new(caller),
        thread: if thread == 0 {
            None
        } else {
            Some(ThreadId::new(thread))
        },
    };
}

/// Returns the runtime context currently used for libc/syscall dispatch.
pub fn mirage_runtime_syscall_context() -> MirageRuntimeSyscallContext {
    unsafe { RUNTIME_SYSCALL_CONTEXT }
}

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

pub fn getpid_syscall<const MAX_PROC: usize, const MSG_DEPTH: usize>(
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

pub fn getppid_syscall<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
) -> KernelResult<ProcessId> {
    syscall(
        kernel,
        caller,
        thread,
        SyscallNumber::GetPpid,
        [0; SYSCALL_MAX_ARGS],
    )
    .map(ProcessId::new)
}

pub fn exit<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    status: i32,
) -> KernelResult<()> {
    syscall(
        kernel,
        caller,
        thread,
        SyscallNumber::Exit,
        [status as u64, 0, 0, 0, 0, 0],
    )
    .map(|_| ())
}

pub fn getuid_syscall<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
) -> KernelResult<u32> {
    syscall(
        kernel,
        caller,
        thread,
        SyscallNumber::GetUid,
        [0; SYSCALL_MAX_ARGS],
    )
    .map(|uid| uid as u32)
}

pub fn getgid_syscall<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
) -> KernelResult<u32> {
    syscall(
        kernel,
        caller,
        thread,
        SyscallNumber::GetGid,
        [0; SYSCALL_MAX_ARGS],
    )
    .map(|gid| gid as u32)
}

pub fn clock_gettime_syscall<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    clock_id: i32,
    out: &mut MirageTimespec,
) -> KernelResult<()> {
    syscall(
        kernel,
        caller,
        thread,
        SyscallNumber::ClockGettime,
        [
            clock_id as u64,
            out as *mut MirageTimespec as u64,
            0,
            0,
            0,
            0,
        ],
    )
    .map(|_| ())
}

pub fn clone_thread<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    entry_point: u64,
    priority: ProcessPriority,
) -> KernelResult<ThreadId> {
    syscall(
        kernel,
        caller,
        thread,
        SyscallNumber::Clone,
        [entry_point, encode_priority(priority), 0, 0, 0, 0],
    )
    .map(ThreadId::new)
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

#[allow(dead_code)]
pub(super) fn raw_syscall_default(
    kernel: *mut DefaultKernel,
    caller: u64,
    number: u64,
    args: [u64; SYSCALL_MAX_ARGS],
) -> isize {
    use crate::kernel::syscall::MIRAGE_EFAULT;

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
        Err(error) => -(errno::libc_errno(error) as isize),
    }
}

pub(super) fn raw_syscall_runtime(number: u64, args: [u64; SYSCALL_MAX_ARGS]) -> isize {
    use crate::kernel::syscall::MIRAGE_EFAULT;

    let context = mirage_runtime_syscall_context();
    if context.kernel.is_null() {
        return -(MIRAGE_EFAULT as isize);
    }

    let result = unsafe {
        (&mut *context.kernel).handle_syscall(
            number,
            SyscallContext::new(context.caller, context.thread, args),
        )
    };
    match result {
        Ok(value) => value as isize,
        Err(error) => -(errno::libc_errno(error) as isize),
    }
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
