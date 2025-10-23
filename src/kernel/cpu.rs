//! CPU topology helpers for the Mirage kernel. The simulated environment keeps
//! track of a handful of virtual cores so the scheduler can distribute work.

use crate::kernel::thread::ThreadId;

pub const MAX_CORES: usize = 4;

#[derive(Clone, Copy, Debug)]
pub struct CpuCoreState {
    pub online: bool,
    pub current_thread: Option<ThreadId>,
    pub local_ticks: u64,
    pub idle_ticks: u64,
}

impl CpuCoreState {
    pub const fn new() -> Self {
        Self {
            online: false,
            current_thread: None,
            local_ticks: 0,
            idle_ticks: 0,
        }
    }

    pub fn online(&mut self) {
        self.online = true;
    }

    pub fn start_thread(&mut self, thread: ThreadId) {
        self.online = true;
        self.current_thread = Some(thread);
    }

    pub fn finish_cycle(&mut self) {
        if self.online {
            self.local_ticks = self.local_ticks.saturating_add(1);
        }
        self.current_thread = None;
    }

    pub fn idle_cycle(&mut self) {
        if self.online {
            self.idle_ticks = self.idle_ticks.saturating_add(1);
        }
        self.current_thread = None;
    }

    pub fn evict(&mut self, thread: ThreadId) {
        if self.current_thread == Some(thread) {
            self.current_thread = None;
        }
    }
}
