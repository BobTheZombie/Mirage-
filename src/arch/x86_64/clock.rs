//! Conceptual hardware clock implementation for the Mirage kernel.
//!
//! The clock keeps track of monotonically increasing ticks that model the
//! platform's programmable interval timer. Even though the kernel does not
//! interact with real hardware, providing a deterministic clock abstraction
//! allows subsystems such as the scheduler to coordinate work across multiple
//! simulated CPU cores.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The default tick frequency used by the conceptual hardware clock.
pub const DEFAULT_FREQUENCY_HZ: u64 = 1_000_000;

/// A minimal model of a multi-core aware hardware clock.
pub struct HardwareClock {
    counter: AtomicU64,
    frequency_hz: AtomicU64,
    calibrated: AtomicBool,
}

impl HardwareClock {
    pub const fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
            frequency_hz: AtomicU64::new(DEFAULT_FREQUENCY_HZ),
            calibrated: AtomicBool::new(false),
        }
    }

    /// Reset the clock tick counter back to zero.
    pub fn reset(&self) {
        self.counter.store(0, Ordering::SeqCst);
    }

    /// Configure the expected tick frequency. The clock keeps running while
    /// the frequency changes, mirroring how a real kernel would adjust the PIT
    /// or HPET divisor at runtime.
    pub fn set_frequency(&self, frequency_hz: u64) {
        let frequency = frequency_hz.max(1);
        self.frequency_hz.store(frequency, Ordering::SeqCst);
    }

    /// Record that the clock has been calibrated against a reference source.
    pub fn mark_calibrated(&self) {
        self.calibrated.store(true, Ordering::SeqCst);
    }

    /// Returns whether the clock has been calibrated.
    pub fn is_calibrated(&self) -> bool {
        self.calibrated.load(Ordering::SeqCst)
    }

    /// Advance the clock by a single tick and return the new tick count.
    pub fn tick(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Advance the clock by `ticks` and return the resulting tick count.
    pub fn advance(&self, ticks: u64) -> u64 {
        if ticks == 0 {
            return self.counter.load(Ordering::SeqCst);
        }
        self.counter.fetch_add(ticks, Ordering::SeqCst) + ticks
    }

    /// Return the current tick counter without modifying it.
    pub fn now(&self) -> u64 {
        self.counter.load(Ordering::SeqCst)
    }

    /// Return the frequency associated with the clock.
    pub fn frequency(&self) -> u64 {
        self.frequency_hz.load(Ordering::SeqCst)
    }
}

/// Global instance of the conceptual hardware clock.
pub static HARDWARE_CLOCK: HardwareClock = HardwareClock::new();
