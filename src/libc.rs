//! User-facing syscall wrappers.
//!
//! These helpers keep libc-style services on the public syscall ABI instead of
//! reaching into kernel internals directly. Synchronous wrappers still call the
//! kernel entry point directly, while [`queue_syscall_trap`] models the register
//! handoff a running thread would perform with the CPU syscall instruction.

use core::ffi::c_void;

use crate::kernel::device::{DeviceId, MirageDeviceDescriptor};
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

#[no_mangle]
pub unsafe extern "C" fn mirage_device_enumerate(
    kernel: *mut Kernel<{ crate::kernel::MAX_PROCESSES }, { crate::kernel::MESSAGE_DEPTH }>,
    caller: u64,
    out: *mut MirageDeviceDescriptor,
    capacity: usize,
) -> isize {
    if kernel.is_null() || (capacity > 0 && out.is_null()) {
        return -1;
    }
    let out_slice = if capacity == 0 {
        &mut []
    } else {
        core::slice::from_raw_parts_mut(out, capacity)
    };
    match enumerate_devices(&mut *kernel, ProcessId::new(caller), None, out_slice) {
        Ok(count) => count as isize,
        Err(_) => -1,
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
        return -1;
    }
    match device_info(
        &mut *kernel,
        ProcessId::new(caller),
        None,
        DeviceId::new(id),
        &mut *out,
    ) {
        Ok(()) => 0,
        Err(_) => -1,
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
        return -1;
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
        Err(_) => -1,
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
        return -1;
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
        Err(_) => -1,
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
