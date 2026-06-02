//! Thread management primitives used by the Mirage kernel scheduler.

use crate::kernel::process::{ProcessId, ProcessPriority, SignalMask};
use crate::kernel::syscall::SYSCALL_MAX_ARGS;

pub const THREADS_PER_PROCESS: usize = 4;
pub const MAX_THREADS: usize = 256;

pub const USER_RFLAGS: u64 = 0x202;
pub const KERNEL_RFLAGS: u64 = 0x202;
pub const KERNEL_CODE_SELECTOR: u64 = 0x08;
pub const KERNEL_DATA_SELECTOR: u64 = 0x10;
pub const USER_CODE_SELECTOR: u64 = 0x1b;
pub const USER_DATA_SELECTOR: u64 = 0x23;
pub const SYSCALL_TRAP_VECTOR: u64 = 0x80;
pub const TIMER_INTERRUPT_VECTOR: u64 = 32;

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

#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivilegeMode {
    Kernel = 0,
    User = 3,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuContext {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
    pub fs: u64,
    pub gs: u64,
    pub trap_vector: u64,
    pub error_code: u64,
    pub privilege_mode: PrivilegeMode,
}

impl CpuContext {
    pub const fn new(
        instruction_pointer: u64,
        stack_pointer: u64,
        privilege_mode: PrivilegeMode,
    ) -> Self {
        let (cs, ss, rflags) = match privilege_mode {
            PrivilegeMode::Kernel => (KERNEL_CODE_SELECTOR, KERNEL_DATA_SELECTOR, KERNEL_RFLAGS),
            PrivilegeMode::User => (USER_CODE_SELECTOR, USER_DATA_SELECTOR, USER_RFLAGS),
        };

        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: instruction_pointer,
            cs,
            rflags,
            rsp: stack_pointer,
            ss,
            fs: 0,
            gs: 0,
            trap_vector: 0,
            error_code: 0,
            privilege_mode,
        }
    }

    pub const fn syscall_number(&self) -> u64 {
        self.rax
    }

    pub const fn syscall_args(&self) -> [u64; SYSCALL_MAX_ARGS] {
        [self.rdi, self.rsi, self.rdx, self.r10, self.r8, self.r9]
    }

    pub fn stage_syscall_trap(&mut self, number: u64, args: [u64; SYSCALL_MAX_ARGS]) {
        self.rax = number;
        self.rdi = args[0];
        self.rsi = args[1];
        self.rdx = args[2];
        self.r10 = args[3];
        self.r8 = args[4];
        self.r9 = args[5];
        self.trap_vector = SYSCALL_TRAP_VECTOR;
        self.error_code = 0;
    }

    pub fn clear_trap(&mut self) {
        self.trap_vector = 0;
        self.error_code = 0;
    }

    pub fn write_syscall_result(&mut self, result: u64) {
        self.rax = result;
        self.clear_trap();
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
    pub signal_mask: SignalMask,
    pub active_signal: Option<u8>,
    pub thread_group: ProcessId,
    pub tls_base: u64,
    pub shares_address_space: bool,
    pub shares_descriptor_table: bool,
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
            signal_mask: SignalMask::EMPTY,
            active_signal: None,
            thread_group: process,
            tls_base: 0,
            shares_address_space: false,
            shares_descriptor_table: false,
        }
    }

    pub fn prepare_syscall(&mut self, number: u64, args: [u64; SYSCALL_MAX_ARGS]) {
        self.context.stage_syscall_trap(number, args);
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

    pub fn replace_exec_image(&mut self, entry_point: u64, stack_pointer: u64) {
        self.entry_point = entry_point;
        self.stack_pointer = stack_pointer;
        self.context = CpuContext::new(entry_point, stack_pointer, PrivilegeMode::User);
        self.tls_base = 0;
        self.state = ThreadState::Ready;
        self.active_signal = None;
    }

    pub fn configure_clone_semantics(
        &mut self,
        thread_group: ProcessId,
        tls_base: u64,
        shares_address_space: bool,
        shares_descriptor_table: bool,
    ) {
        self.thread_group = thread_group;
        self.tls_base = tls_base;
        self.context.fs = tls_base;
        self.shares_address_space = shares_address_space;
        self.shares_descriptor_table = shares_descriptor_table;
    }

    pub fn set_signal_mask(&mut self, mask: SignalMask) -> SignalMask {
        let previous = self.signal_mask;
        self.signal_mask = mask;
        previous
    }

    pub fn deliver_signal(&mut self, signal: u8, handler: u64) {
        self.active_signal = Some(signal);
        self.context.rdi = signal as u64;
        if handler != 0 {
            self.context.rip = handler;
        }
    }

    pub fn finish_signal(&mut self) {
        self.active_signal = None;
    }

    pub fn accumulate_cpu_time(&mut self, ticks: u64) {
        self.cpu_time = self.cpu_time.saturating_add(ticks as u128);
    }
}
