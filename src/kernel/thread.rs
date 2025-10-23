//! Thread management primitives used by the Mirage kernel scheduler.

use crate::kernel::process::{ProcessId, ProcessPriority};

pub const THREADS_PER_PROCESS: usize = 4;
pub const MAX_THREADS: usize = 256;

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

#[derive(Clone, Copy, Debug)]
pub struct ThreadControlBlock {
    pub id: ThreadId,
    pub process: ProcessId,
    pub priority: ProcessPriority,
    pub state: ThreadState,
    pub entry_point: u64,
    pub stack_pointer: u64,
    pub cpu_time: u128,
}

impl ThreadControlBlock {
    pub const fn new(
        id: ThreadId,
        process: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
    ) -> Self {
        Self {
            id,
            process,
            priority,
            state: ThreadState::Ready,
            entry_point,
            stack_pointer: 0,
            cpu_time: 0,
        }
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
