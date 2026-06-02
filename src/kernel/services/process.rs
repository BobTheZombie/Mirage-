//! Process and thread lifecycle service seam.

use crate::kernel::ipc::{Message, MessagePayload};
use crate::kernel::process::{ExitStatus, ProcessId, ProcessPriority};
use crate::kernel::syscall::{SyscallContext, SyscallNumber, SYSCALL_MAX_ARGS};
use crate::kernel::thread::{CpuContext, ThreadId};
use crate::kernel::{Kernel, KernelResult};
use crate::subkernel::Credentials;

/// Kernel-internal adapter for process, thread, and IPC lifecycle operations.
pub trait ProcessService {
    fn spawn_initial_process(&mut self, creds: Credentials) -> KernelResult<ProcessId>;

    fn spawn_child_process(
        &mut self,
        parent_pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
        requested_creds: Credentials,
    ) -> KernelResult<ProcessId>;

    fn spawn_thread(
        &mut self,
        pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
    ) -> KernelResult<ThreadId>;

    fn exit_process(&mut self, pid: ProcessId, status: ExitStatus);

    fn terminate_process(&mut self, pid: ProcessId);

    fn terminate_thread(&mut self, thread: ThreadId);

    fn wait(&mut self, parent: ProcessId, status: Option<&mut i32>) -> KernelResult<ProcessId>;

    fn waitpid(
        &mut self,
        parent: ProcessId,
        selector: i64,
        status: Option<&mut i32>,
        options: u64,
    ) -> KernelResult<ProcessId>;

    fn send_message(
        &mut self,
        sender: ProcessId,
        receiver: ProcessId,
        payload: MessagePayload,
    ) -> KernelResult<()>;

    fn receive_message(&mut self, pid: ProcessId) -> KernelResult<Message>;

    fn receive_or_block(&mut self, pid: ProcessId) -> KernelResult<Option<Message>>;

    fn queue_thread_syscall(
        &mut self,
        thread: ThreadId,
        number: u64,
        args: [u64; SYSCALL_MAX_ARGS],
    ) -> KernelResult<()>;

    fn thread_context(&self, thread: ThreadId) -> KernelResult<CpuContext>;

    fn clone_thread_via_abi(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        entry_point: u64,
        priority: ProcessPriority,
    ) -> KernelResult<ThreadId>;

    fn exit_via_abi(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        status: i32,
    ) -> KernelResult<()>;
}

impl<const MAX_PROC: usize, const MSG_DEPTH: usize> ProcessService for Kernel<MAX_PROC, MSG_DEPTH> {
    fn spawn_initial_process(&mut self, creds: Credentials) -> KernelResult<ProcessId> {
        Kernel::spawn_initial_process(self, creds)
    }

    fn spawn_child_process(
        &mut self,
        parent_pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
        requested_creds: Credentials,
    ) -> KernelResult<ProcessId> {
        Kernel::spawn_child_process(self, parent_pid, entry_point, priority, requested_creds)
    }

    fn spawn_thread(
        &mut self,
        pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
    ) -> KernelResult<ThreadId> {
        Kernel::spawn_thread(self, pid, entry_point, priority)
    }

    fn exit_process(&mut self, pid: ProcessId, status: ExitStatus) {
        Kernel::exit_process(self, pid, status);
    }

    fn terminate_process(&mut self, pid: ProcessId) {
        Kernel::terminate_process(self, pid);
    }

    fn terminate_thread(&mut self, thread: ThreadId) {
        Kernel::terminate_thread(self, thread);
    }

    fn wait(&mut self, parent: ProcessId, status: Option<&mut i32>) -> KernelResult<ProcessId> {
        Kernel::wait(self, parent, status)
    }

    fn waitpid(
        &mut self,
        parent: ProcessId,
        selector: i64,
        status: Option<&mut i32>,
        options: u64,
    ) -> KernelResult<ProcessId> {
        Kernel::waitpid(self, parent, selector, status, options)
    }

    fn send_message(
        &mut self,
        sender: ProcessId,
        receiver: ProcessId,
        payload: MessagePayload,
    ) -> KernelResult<()> {
        Kernel::send_message(self, sender, receiver, payload)
    }

    fn receive_message(&mut self, pid: ProcessId) -> KernelResult<Message> {
        Kernel::receive_message(self, pid)
    }

    fn receive_or_block(&mut self, pid: ProcessId) -> KernelResult<Option<Message>> {
        Kernel::receive_or_block(self, pid)
    }

    fn queue_thread_syscall(
        &mut self,
        thread: ThreadId,
        number: u64,
        args: [u64; SYSCALL_MAX_ARGS],
    ) -> KernelResult<()> {
        Kernel::queue_thread_syscall(self, thread, number, args)
    }

    fn thread_context(&self, thread: ThreadId) -> KernelResult<CpuContext> {
        Kernel::thread_context(self, thread)
    }

    fn clone_thread_via_abi(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        entry_point: u64,
        priority: ProcessPriority,
    ) -> KernelResult<ThreadId> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::Clone,
            [entry_point, encode_priority(priority), 0, 0, 0, 0],
        )
        .map(ThreadId::new)
    }

    fn exit_via_abi(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        status: i32,
    ) -> KernelResult<()> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::Exit,
            [status as u64, 0, 0, 0, 0, 0],
        )
        .map(|_| ())
    }
}

fn service_syscall<const MAX_PROC: usize, const MSG_DEPTH: usize>(
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
