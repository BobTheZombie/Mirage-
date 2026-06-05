//! Portable MTSS run-queue mechanics.
//!
//! This module contains the fixed-capacity, allocation-free queue core used by
//! Mirage scheduler integrations. It intentionally stores caller-provided
//! thread, process, and priority identifiers generically so kernel-side types do
//! not leak into the MTSS crate.

use crate::MtssError;

/// MTSS scheduling record for one runnable micro-thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MtssThreadScheduleRecord<Thread, Process, Priority> {
    pub thread: Thread,
    pub process: Process,
    pub priority: Priority,
    remaining_slice: u8,
    slice_budget: u8,
}

impl<Thread, Process, Priority> MtssThreadScheduleRecord<Thread, Process, Priority> {
    pub const fn new(
        thread: Thread,
        process: Process,
        priority: Priority,
        slice_budget: u8,
    ) -> Self {
        Self {
            thread,
            process,
            priority,
            remaining_slice: slice_budget,
            slice_budget,
        }
    }

    pub const fn remaining_slice(&self) -> u8 {
        self.remaining_slice
    }

    pub const fn slice_budget(&self) -> u8 {
        self.slice_budget
    }

    pub fn consume_time_slice(&mut self) -> bool {
        if self.remaining_slice > 0 {
            self.remaining_slice -= 1;
        }
        self.remaining_slice == 0
    }

    pub fn reset_time_slice(&mut self) {
        self.remaining_slice = self.slice_budget;
    }
}

/// Fixed-capacity MTSS run queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunQueue<Record, const MAX: usize> {
    queue: [Option<Record>; MAX],
    head: usize,
    tail: usize,
    len: usize,
}

impl<Record: Copy, const MAX: usize> RunQueue<Record, MAX> {
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

    pub fn enqueue(&mut self, record: Record) -> Result<(), MtssError> {
        if self.len == MAX {
            return Err(MtssError::RunQueueFull);
        }
        self.queue[self.tail] = Some(record);
        self.tail = (self.tail + 1) % MAX;
        self.len += 1;
        Ok(())
    }

    pub fn requeue(&mut self, record: Record) -> Result<(), MtssError> {
        self.enqueue(record)
    }

    pub fn next(&mut self) -> Option<Record> {
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

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub fn remove_matching(&mut self, mut matches: impl FnMut(Record) -> bool) -> usize {
        let mut removed = 0usize;
        let mut idx = 0;
        while idx < MAX {
            if let Some(entry) = self.queue[idx] {
                if matches(entry) {
                    self.queue[idx] = None;
                    if self.len > 0 {
                        self.len -= 1;
                    }
                    removed += 1;
                }
            }
            idx += 1;
        }
        removed
    }
}

impl<Thread, Process, Priority, const MAX: usize>
    RunQueue<MtssThreadScheduleRecord<Thread, Process, Priority>, MAX>
where
    Thread: Copy + PartialEq,
    Process: Copy + PartialEq,
    Priority: Copy,
{
    pub fn contains_process(&self, process: Process) -> bool {
        let mut idx = 0;
        while idx < MAX {
            if let Some(entry) = self.queue[idx] {
                if entry.process == process {
                    return true;
                }
            }
            idx += 1;
        }
        false
    }

    pub fn remove_thread(&mut self, thread: Thread) -> usize {
        self.remove_matching(|entry| entry.thread == thread)
    }

    pub fn remove_process(&mut self, process: Process) -> usize {
        self.remove_matching(|entry| entry.process == process)
    }
}
