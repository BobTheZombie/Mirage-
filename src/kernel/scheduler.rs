//! A simple multi-level round-robin scheduler for the Mirage kernel.

use crate::kernel::process::{ProcessId, ProcessPriority};
use crate::kernel::thread::ThreadId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerError {
    QueueFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduledThread {
    pub thread: ThreadId,
    pub process: ProcessId,
    pub priority: ProcessPriority,
    remaining_slice: u8,
}

impl ScheduledThread {
    pub const fn new(thread: ThreadId, process: ProcessId, priority: ProcessPriority) -> Self {
        Self {
            thread,
            process,
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
    queue: [Option<ScheduledThread>; MAX],
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

    pub fn enqueue(&mut self, thread: ScheduledThread) -> Result<(), SchedulerError> {
        if self.len == MAX {
            return Err(SchedulerError::QueueFull);
        }
        self.queue[self.tail] = Some(thread);
        self.tail = (self.tail + 1) % MAX;
        self.len += 1;
        Ok(())
    }

    pub fn requeue(&mut self, thread: ScheduledThread) -> Result<(), SchedulerError> {
        self.enqueue(thread)
    }

    pub fn next(&mut self) -> Option<ScheduledThread> {
        if self.len == 0 {
            return None;
        }

        let mut steps = 0;
        while steps < MAX {
            let idx = (self.head + steps) % MAX;
            if let Some(entry) = self.queue[idx] {
                self.queue[idx] = None;
                self.len -= 1;
                self.head = (idx + 1) % MAX;
                return Some(entry);
            }
            steps += 1;
        }

        self.head = 0;
        self.tail = 0;
        self.len = 0;
        None
    }

    pub fn remove_thread(&mut self, thread: ThreadId) {
        let mut idx = 0;
        while idx < MAX {
            if let Some(entry) = self.queue[idx] {
                if entry.thread == thread {
                    self.queue[idx] = None;
                    if self.len > 0 {
                        self.len -= 1;
                    }
                }
            }
            idx += 1;
        }
    }

    pub fn remove_process(&mut self, process: ProcessId) {
        let mut idx = 0;
        while idx < MAX {
            if let Some(entry) = self.queue[idx] {
                if entry.process == process {
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
