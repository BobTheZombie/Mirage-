//! Timekeeping utilities layered on top of the architecture specific hardware
//! clock abstraction.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::x86_64::clock::HARDWARE_CLOCK;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonotonicTimestamp {
    ticks: u64,
    frequency_hz: u64,
}

impl MonotonicTimestamp {
    pub const fn new(ticks: u64, frequency_hz: u64) -> Self {
        Self {
            ticks,
            frequency_hz,
        }
    }

    pub const fn ticks(&self) -> u64 {
        self.ticks
    }

    pub const fn frequency(&self) -> u64 {
        self.frequency_hz
    }

    pub fn as_nanos(&self) -> u128 {
        if self.frequency_hz == 0 {
            return 0;
        }
        (self.ticks as u128 * 1_000_000_000u128) / self.frequency_hz as u128
    }

    pub fn as_micros(&self) -> u128 {
        if self.frequency_hz == 0 {
            return 0;
        }
        (self.ticks as u128 * 1_000_000u128) / self.frequency_hz as u128
    }
}

pub struct KernelTime {
    last_tick: AtomicU64,
}

impl KernelTime {
    pub const fn new() -> Self {
        Self {
            last_tick: AtomicU64::new(0),
        }
    }

    pub fn init(&self, frequency_hz: u64) {
        HARDWARE_CLOCK.set_frequency(frequency_hz);
        HARDWARE_CLOCK.reset();
        HARDWARE_CLOCK.mark_calibrated();
        self.last_tick.store(0, Ordering::SeqCst);
    }

    pub fn tick(&self) -> MonotonicTimestamp {
        let ticks = HARDWARE_CLOCK.tick();
        self.last_tick.store(ticks, Ordering::SeqCst);
        MonotonicTimestamp::new(ticks, HARDWARE_CLOCK.frequency())
    }

    pub fn advance_ticks(&self, ticks: u64) -> MonotonicTimestamp {
        let total = HARDWARE_CLOCK.advance(ticks);
        self.last_tick.store(total, Ordering::SeqCst);
        MonotonicTimestamp::new(total, HARDWARE_CLOCK.frequency())
    }

    pub fn now(&self) -> MonotonicTimestamp {
        let ticks = HARDWARE_CLOCK.now();
        MonotonicTimestamp::new(ticks, HARDWARE_CLOCK.frequency())
    }

    pub fn uptime_ticks(&self) -> u64 {
        HARDWARE_CLOCK.now()
    }
}

pub static KERNEL_TIME: KernelTime = KernelTime::new();
