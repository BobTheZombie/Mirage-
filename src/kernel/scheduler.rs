//! A simple multi-level round-robin scheduler for the Mirage kernel.

use crate::kernel::process::{ProcessId, ProcessPriority};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerError {
    QueueFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduledProcess {
    pub pid: ProcessId,
    pub priority: ProcessPriority,
    remaining_slice: u8,
}

impl ScheduledProcess {
    pub const fn new(pid: ProcessId, priority: ProcessPriority) -> Self {
        Self {
            pid,
            priority,
            remaining_slice: priority.time_slice(),
        }
    }

    pub fn consume_time_slice(&mut self) -> bool {
        if self.remaining_slice > 0 {
            self.remaining_slice -= 1;
        }
        self.remaining_slice == 0
    }

    pub fn reset_time_slice(&mut self) {
        self.remaining_slice = self.priority.time_slice();
    }
}

#[derive(Clone, Copy)]
pub struct Scheduler<const MAX: usize> {
    queue: [Option<ScheduledProcess>; MAX],
    head: usize,
    tail: usize,
    len: usize,
}

impl<const MAX: usize> Scheduler<MAX> {
    pub const fn new() -> Self {
        Self {
            queue: [None; MAX],
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    pub fn reset(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.len = 0;
        let mut idx = 0;
        while idx < MAX {
            self.queue[idx] = None;
            idx += 1;
        }
    }

    pub fn enqueue(&mut self, process: ScheduledProcess) -> Result<(), SchedulerError> {
        if self.len == MAX {
            return Err(SchedulerError::QueueFull);
        }
        self.queue[self.tail] = Some(process);
        self.tail = (self.tail + 1) % MAX;
        self.len += 1;
        Ok(())
    }

    pub fn requeue(&mut self, process: ScheduledProcess) -> Result<(), SchedulerError> {
        self.enqueue(process)
    }

    pub fn next(&mut self) -> Option<ScheduledProcess> {
        if self.len == 0 {
            return None;
        }

        let mut steps = 0;
        while steps < MAX {
            let idx = (self.head + steps) % MAX;
            if let Some(proc) = self.queue[idx] {
                self.queue[idx] = None;
                self.len -= 1;
                self.head = (idx + 1) % MAX;
                return Some(proc);
            }
            steps += 1;
        }

        self.head = 0;
        self.tail = 0;
        self.len = 0;
        None
    }

    pub fn remove(&mut self, pid: ProcessId) {
        let mut idx = 0;
        while idx < MAX {
            if let Some(entry) = self.queue[idx] {
                if entry.pid == pid {
                    self.queue[idx] = None;
                    if self.len > 0 {
                        self.len -= 1;
                    }
                }
            }
            idx += 1;
        }
    }
}
