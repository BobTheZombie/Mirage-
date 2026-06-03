//! Fixed-capacity sleep and process timer tracking for the kernel.

use crate::kernel::process::ProcessId;
use crate::kernel::thread::ThreadId;

pub const MAX_SLEEP_ENTRIES: usize = crate::kernel::thread::MAX_THREADS;
pub const MAX_PROCESS_TIMERS: usize = crate::kernel::MAX_PROCESSES * 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SleepEntry {
    pub process: ProcessId,
    pub thread: Option<ThreadId>,
    pub wake_deadline_ns: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpiredSleep {
    pub process: ProcessId,
    pub thread: Option<ThreadId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessTimer {
    pub owner: ProcessId,
    pub id: u64,
    pub armed: bool,
    pub wake_deadline_ns: u128,
    pub interval_ns: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpiredTimer {
    pub owner: ProcessId,
    pub id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerError {
    Full,
    InvalidTimer,
}

#[derive(Clone, Copy)]
pub struct TimerManager<const SLEEP_CAP: usize, const TIMER_CAP: usize> {
    sleeps: [Option<SleepEntry>; SLEEP_CAP],
    timers: [Option<ProcessTimer>; TIMER_CAP],
    next_timer_id: u64,
}

impl<const SLEEP_CAP: usize, const TIMER_CAP: usize> TimerManager<SLEEP_CAP, TIMER_CAP> {
    pub const fn new() -> Self {
        Self {
            sleeps: [None; SLEEP_CAP],
            timers: [None; TIMER_CAP],
            next_timer_id: 1,
        }
    }

    pub fn reset(&mut self) {
        let mut idx = 0usize;
        while idx < SLEEP_CAP {
            self.sleeps[idx] = None;
            idx += 1;
        }
        idx = 0;
        while idx < TIMER_CAP {
            self.timers[idx] = None;
            idx += 1;
        }
        self.next_timer_id = 1;
    }

    pub fn add_sleep(
        &mut self,
        process: ProcessId,
        thread: Option<ThreadId>,
        wake_deadline_ns: u128,
    ) -> Result<(), TimerError> {
        let mut idx = 0usize;
        while idx < SLEEP_CAP {
            if self.sleeps[idx].is_none() {
                self.sleeps[idx] = Some(SleepEntry {
                    process,
                    thread,
                    wake_deadline_ns,
                });
                return Ok(());
            }
            idx += 1;
        }
        Err(TimerError::Full)
    }

    pub fn expire_sleep(&mut self, now_ns: u128) -> Option<ExpiredSleep> {
        let mut idx = 0usize;
        while idx < SLEEP_CAP {
            if let Some(entry) = self.sleeps[idx] {
                if entry.wake_deadline_ns <= now_ns {
                    self.sleeps[idx] = None;
                    return Some(ExpiredSleep {
                        process: entry.process,
                        thread: entry.thread,
                    });
                }
            }
            idx += 1;
        }
        None
    }

    pub fn create_timer(&mut self, owner: ProcessId) -> Result<u64, TimerError> {
        let mut idx = 0usize;
        while idx < TIMER_CAP {
            if self.timers[idx].is_none() {
                let id = self.allocate_timer_id(owner);
                self.timers[idx] = Some(ProcessTimer {
                    owner,
                    id,
                    armed: false,
                    wake_deadline_ns: 0,
                    interval_ns: 0,
                });
                return Ok(id);
            }
            idx += 1;
        }
        Err(TimerError::Full)
    }

    pub fn timer(&self, owner: ProcessId, id: u64) -> Result<ProcessTimer, TimerError> {
        let idx = self.locate_timer(owner, id)?;
        self.timers[idx].ok_or(TimerError::InvalidTimer)
    }

    pub fn set_timer(
        &mut self,
        owner: ProcessId,
        id: u64,
        deadline_ns: Option<u128>,
        interval_ns: u128,
    ) -> Result<ProcessTimer, TimerError> {
        let idx = self.locate_timer(owner, id)?;
        let timer = self.timers[idx].as_mut().ok_or(TimerError::InvalidTimer)?;
        let previous = *timer;
        timer.interval_ns = interval_ns;
        if let Some(deadline) = deadline_ns {
            timer.armed = true;
            timer.wake_deadline_ns = deadline;
        } else {
            timer.armed = false;
            timer.wake_deadline_ns = 0;
        }
        Ok(previous)
    }

    pub fn delete_timer(&mut self, owner: ProcessId, id: u64) -> Result<(), TimerError> {
        let idx = self.locate_timer(owner, id)?;
        self.timers[idx] = None;
        Ok(())
    }

    pub fn release_process(&mut self, owner: ProcessId) {
        let mut idx = 0usize;
        while idx < SLEEP_CAP {
            if let Some(entry) = self.sleeps[idx] {
                if entry.process == owner {
                    self.sleeps[idx] = None;
                }
            }
            idx += 1;
        }
        idx = 0;
        while idx < TIMER_CAP {
            if let Some(timer) = self.timers[idx] {
                if timer.owner == owner {
                    self.timers[idx] = None;
                }
            }
            idx += 1;
        }
    }

    pub fn expire_timer(&mut self, now_ns: u128) -> Option<ExpiredTimer> {
        let mut idx = 0usize;
        while idx < TIMER_CAP {
            if let Some(mut timer) = self.timers[idx] {
                if timer.armed && timer.wake_deadline_ns <= now_ns {
                    if timer.interval_ns > 0 {
                        let elapsed = now_ns.saturating_sub(timer.wake_deadline_ns);
                        let missed_periods = elapsed / timer.interval_ns + 1;
                        timer.wake_deadline_ns = timer
                            .wake_deadline_ns
                            .saturating_add(timer.interval_ns.saturating_mul(missed_periods));
                        self.timers[idx] = Some(timer);
                    } else {
                        timer.armed = false;
                        timer.wake_deadline_ns = 0;
                        self.timers[idx] = Some(timer);
                    }
                    return Some(ExpiredTimer {
                        owner: timer.owner,
                        id: timer.id,
                    });
                }
            }
            idx += 1;
        }
        None
    }

    fn locate_timer(&self, owner: ProcessId, id: u64) -> Result<usize, TimerError> {
        let mut idx = 0usize;
        while idx < TIMER_CAP {
            if let Some(timer) = self.timers[idx] {
                if timer.owner == owner && timer.id == id {
                    return Ok(idx);
                }
            }
            idx += 1;
        }
        Err(TimerError::InvalidTimer)
    }

    fn allocate_timer_id(&mut self, owner: ProcessId) -> u64 {
        let start = self.next_timer_id.max(1);
        let mut candidate = start;
        loop {
            if self.locate_timer(owner, candidate).is_err() {
                self.next_timer_id = candidate.wrapping_add(1).max(1);
                return candidate;
            }
            candidate = candidate.wrapping_add(1).max(1);
            if candidate == start {
                // Exhaustion is practically unreachable with fixed slots; fall back to 1.
                return 1;
            }
        }
    }
}
