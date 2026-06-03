//! Fixed-capacity futex wait queues keyed by user address and address space.

use crate::kernel::thread::ThreadId;

pub const MAX_FUTEX_WAITERS: usize = crate::kernel::thread::MAX_THREADS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FutexKey {
    pub owner: u64,
    pub address: u64,
}

impl FutexKey {
    pub const fn new(owner: u64, address: u64) -> Self {
        Self { owner, address }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FutexWaiter {
    pub key: FutexKey,
    pub thread: ThreadId,
    pub deadline_ns: Option<u128>,
}

impl FutexWaiter {
    pub const fn new(key: FutexKey, thread: ThreadId, deadline_ns: Option<u128>) -> Self {
        Self {
            key,
            thread,
            deadline_ns,
        }
    }
}

#[derive(Clone, Copy)]
pub struct FutexTable<const MAX: usize> {
    waiters: [Option<FutexWaiter>; MAX],
}

impl<const MAX: usize> FutexTable<MAX> {
    pub const fn new() -> Self {
        Self {
            waiters: [None; MAX],
        }
    }

    pub fn reset(&mut self) {
        let mut idx = 0usize;
        while idx < MAX {
            self.waiters[idx] = None;
            idx += 1;
        }
    }

    pub fn enqueue(
        &mut self,
        key: FutexKey,
        thread: ThreadId,
        deadline_ns: Option<u128>,
    ) -> Result<(), FutexTableError> {
        let mut idx = 0usize;
        let mut free = None;
        while idx < MAX {
            match self.waiters[idx] {
                Some(waiter) if waiter.thread == thread => {
                    self.waiters[idx] = Some(FutexWaiter::new(key, thread, deadline_ns));
                    return Ok(());
                }
                None if free.is_none() => free = Some(idx),
                _ => {}
            }
            idx += 1;
        }

        let slot = free.ok_or(FutexTableError::Full)?;
        self.waiters[slot] = Some(FutexWaiter::new(key, thread, deadline_ns));
        Ok(())
    }

    pub fn wake(&mut self, key: FutexKey, limit: usize, out: &mut [Option<ThreadId>]) -> usize {
        if limit == 0 || out.is_empty() {
            return 0;
        }
        let mut count = 0usize;
        let mut idx = 0usize;
        while idx < MAX && count < limit && count < out.len() {
            if let Some(waiter) = self.waiters[idx] {
                if waiter.key == key {
                    self.waiters[idx] = None;
                    out[count] = Some(waiter.thread);
                    count += 1;
                }
            }
            idx += 1;
        }
        count
    }

    pub fn expire(&mut self, now_ns: u128, out: &mut [Option<ThreadId>]) -> usize {
        if out.is_empty() {
            return 0;
        }
        let mut count = 0usize;
        let mut idx = 0usize;
        while idx < MAX && count < out.len() {
            if let Some(waiter) = self.waiters[idx] {
                if let Some(deadline) = waiter.deadline_ns {
                    if deadline <= now_ns {
                        self.waiters[idx] = None;
                        out[count] = Some(waiter.thread);
                        count += 1;
                    }
                }
            }
            idx += 1;
        }
        count
    }

    pub fn remove_thread(&mut self, thread: ThreadId) {
        let mut idx = 0usize;
        while idx < MAX {
            if let Some(waiter) = self.waiters[idx] {
                if waiter.thread == thread {
                    self.waiters[idx] = None;
                }
            }
            idx += 1;
        }
    }

    pub fn remove_owner(&mut self, owner: u64) {
        let mut idx = 0usize;
        while idx < MAX {
            if let Some(waiter) = self.waiters[idx] {
                if waiter.key.owner == owner {
                    self.waiters[idx] = None;
                }
            }
            idx += 1;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FutexTableError {
    Full,
}
