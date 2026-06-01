//! User-facing syscall wrappers.
//!
//! These helpers keep libc-style services on the public syscall ABI instead of
//! reaching into kernel internals directly. Synchronous wrappers still call the
//! kernel entry point directly, while [`queue_syscall_trap`] models the register
//! handoff a running thread would perform with the CPU syscall instruction.

use core::ffi::c_void;

use crate::kernel::device::{DeviceDescriptor, DeviceId};
use crate::kernel::ipc::Message;
use crate::kernel::memory::MemoryProtection;
use crate::kernel::process::{ProcessId, ProcessPriority};
use crate::kernel::syscall::{SyscallContext, SyscallNumber, SYSCALL_MAX_ARGS};
use crate::kernel::thread::ThreadId;
use crate::kernel::{Kernel, KernelResult};
use crate::subkernel::SecurityClass;

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
    let args = [out as *mut Message as u64, 0, 0, 0, 0, 0];
    syscall(kernel, caller, thread, SyscallNumber::ReceiveIpc, args).map(|read| read as usize)
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
    out: &mut [DeviceDescriptor],
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
