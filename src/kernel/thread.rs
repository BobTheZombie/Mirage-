//! Thread management primitives used by the Mirage kernel scheduler.

use crate::kernel::process::{ProcessId, ProcessPriority};
use crate::kernel::syscall::SYSCALL_MAX_ARGS;

pub const THREADS_PER_PROCESS: usize = 4;
pub const MAX_THREADS: usize = 256;

pub const USER_RFLAGS: u64 = 0x202;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThreadId(u64);

impl ThreadId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(&self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadState {
    Ready,
    Running,
    Blocked,
    Terminated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivilegeMode {
    Kernel,
    User,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuContext {
    pub instruction_pointer: u64,
    pub stack_pointer: u64,
    pub flags: u64,
    pub syscall_number: u64,
    pub argument_registers: [u64; SYSCALL_MAX_ARGS],
    pub result_register: u64,
    pub privilege_mode: PrivilegeMode,
    pub pending_syscall: bool,
}

impl CpuContext {
    pub const fn new(
        instruction_pointer: u64,
        stack_pointer: u64,
        privilege_mode: PrivilegeMode,
    ) -> Self {
        Self {
            instruction_pointer,
            stack_pointer,
            flags: USER_RFLAGS,
            syscall_number: 0,
            argument_registers: [0; SYSCALL_MAX_ARGS],
            result_register: 0,
            privilege_mode,
            pending_syscall: false,
        }
    }

    pub fn queue_syscall(&mut self, number: u64, args: [u64; SYSCALL_MAX_ARGS]) {
        self.syscall_number = number;
        self.argument_registers = args;
        self.pending_syscall = true;
    }

    pub fn take_syscall(&mut self) -> Option<(u64, [u64; SYSCALL_MAX_ARGS])> {
        if self.pending_syscall {
            self.pending_syscall = false;
            Some((self.syscall_number, self.argument_registers))
        } else {
            None
        }
    }

    pub fn write_syscall_result(&mut self, result: u64) {
        self.result_register = result;
        self.syscall_number = result;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ThreadControlBlock {
    pub id: ThreadId,
    pub process: ProcessId,
    pub priority: ProcessPriority,
    pub state: ThreadState,
    pub entry_point: u64,
    pub stack_pointer: u64,
    pub context: CpuContext,
    pub cpu_time: u128,
}

impl ThreadControlBlock {
    pub const fn new(
        id: ThreadId,
        process: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
        stack_pointer: u64,
    ) -> Self {
        Self {
            id,
            process,
            priority,
            state: ThreadState::Ready,
            entry_point,
            stack_pointer,
            context: CpuContext::new(entry_point, stack_pointer, PrivilegeMode::User),
            cpu_time: 0,
        }
    }

    pub fn prepare_syscall(&mut self, number: u64, args: [u64; SYSCALL_MAX_ARGS]) {
        self.context.queue_syscall(number, args);
    }

    pub fn write_syscall_result(&mut self, result: u64) {
        self.context.write_syscall_result(result);
    }

    pub fn mark_running(&mut self) {
        self.state = ThreadState::Running;
    }

    pub fn mark_ready(&mut self) {
        self.state = ThreadState::Ready;
    }

    pub fn block(&mut self) {
        self.state = ThreadState::Blocked;
    }

    pub fn terminate(&mut self) {
        self.state = ThreadState::Terminated;
    }

    pub fn accumulate_cpu_time(&mut self, ticks: u64) {
        self.cpu_time = self.cpu_time.saturating_add(ticks as u128);
    }
}
