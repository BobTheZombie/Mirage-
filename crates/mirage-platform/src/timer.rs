//! Mechanism-only platform timer selection and calibration.
//!
//! This module models timer facts for early AMD64 platform setup without
//! owning scheduler policy. It chooses a usable clock source by architectural
//! capability and discovery data, then exposes monotonic/frequency primitives
//! that higher layers may consume.

use mirage_amd64::{AmdCpuId, AmdFeatureSet};
use mirage_ryzen::{RyzenFeatureProfile, RyzenPlatform, RyzenQuirk, RyzenSupportStatus};

/// Maximum number of polls used by calibration loops before reporting timeout.
pub const DEFAULT_CALIBRATION_POLL_LIMIT: u32 = 1024;

/// Platform timer errors surfaced as mechanism failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerError {
    NoTimerAvailable,
    InvalidFrequency,
    CalibrationTimeout,
    CalibrationUnderflow,
}

/// Stable identifier for the selected timer mechanism.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum TimerKind {
    Tsc,
    Apic,
    Hpet,
    PitFallback,
}

/// Common operations every Mirage platform timer must expose.
pub trait PlatformTimer {
    fn kind(&self) -> TimerKind;
    fn monotonic_now(&self) -> u64;
    fn timer_frequency(&self) -> u64;

    fn calibrate_timer(&mut self) -> Result<(), TimerError> {
        if self.timer_frequency() == 0 {
            Err(TimerError::InvalidFrequency)
        } else {
            Ok(())
        }
    }
}

/// Read-only TSC counter source used by polling calibration.
pub trait TscCounter {
    fn read_tsc(&mut self) -> u64;
}

/// Reference timer source used while calibrating counters.
pub trait ReferenceTimer {
    fn now_ns(&mut self) -> u64;
}

/// Completed TSC calibration sample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TscCalibration {
    pub start_tsc: u64,
    pub end_tsc: u64,
    pub elapsed_ns: u64,
}

impl TscCalibration {
    pub const fn new(start_tsc: u64, end_tsc: u64, elapsed_ns: u64) -> Self {
        Self {
            start_tsc,
            end_tsc,
            elapsed_ns,
        }
    }

    pub const fn frequency_hz(self) -> Result<u64, TimerError> {
        if self.elapsed_ns == 0 {
            return Err(TimerError::InvalidFrequency);
        }
        if self.end_tsc <= self.start_tsc {
            return Err(TimerError::CalibrationUnderflow);
        }
        let ticks = self.end_tsc - self.start_tsc;
        Ok(((ticks as u128 * 1_000_000_000u128) / self.elapsed_ns as u128) as u64)
    }
}

/// Invariant TSC-backed monotonic timer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TscTimer {
    base_tsc: u64,
    frequency_hz: u64,
    current_tsc: u64,
}

impl TscTimer {
    pub const fn new(base_tsc: u64, frequency_hz: u64) -> Result<Self, TimerError> {
        if frequency_hz == 0 {
            Err(TimerError::InvalidFrequency)
        } else {
            Ok(Self {
                base_tsc,
                frequency_hz,
                current_tsc: base_tsc,
            })
        }
    }

    pub const fn from_calibration(calibration: TscCalibration) -> Result<Self, TimerError> {
        match calibration.frequency_hz() {
            Ok(frequency_hz) => Self::new(calibration.end_tsc, frequency_hz),
            Err(error) => Err(error),
        }
    }

    pub fn calibrate_with_polling(
        tsc: &mut impl TscCounter,
        reference: &mut impl ReferenceTimer,
        target_elapsed_ns: u64,
        poll_limit: u32,
    ) -> Result<Self, TimerError> {
        if target_elapsed_ns == 0 || poll_limit == 0 {
            return Err(TimerError::InvalidFrequency);
        }

        let start_ns = reference.now_ns();
        let start_tsc = tsc.read_tsc();
        let mut polls = 0;
        while polls < poll_limit {
            let now_ns = reference.now_ns();
            let elapsed_ns = now_ns.saturating_sub(start_ns);
            if elapsed_ns >= target_elapsed_ns {
                let end_tsc = tsc.read_tsc();
                return Self::from_calibration(TscCalibration::new(start_tsc, end_tsc, elapsed_ns));
            }
            polls += 1;
        }

        Err(TimerError::CalibrationTimeout)
    }

    pub const fn with_current_tsc(mut self, current_tsc: u64) -> Self {
        self.current_tsc = current_tsc;
        self
    }
}

impl PlatformTimer for TscTimer {
    fn kind(&self) -> TimerKind {
        TimerKind::Tsc
    }

    fn monotonic_now(&self) -> u64 {
        ticks_to_ns(
            self.current_tsc.saturating_sub(self.base_tsc),
            self.frequency_hz,
        )
    }

    fn timer_frequency(&self) -> u64 {
        self.frequency_hz
    }
}

/// Local APIC timer descriptor discovered by AMD64 platform setup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApicTimer {
    frequency_hz: u64,
    elapsed_ticks: u64,
}

impl ApicTimer {
    pub const fn new(frequency_hz: u64) -> Result<Self, TimerError> {
        if frequency_hz == 0 {
            Err(TimerError::InvalidFrequency)
        } else {
            Ok(Self {
                frequency_hz,
                elapsed_ticks: 0,
            })
        }
    }

    pub const fn with_elapsed_ticks(mut self, elapsed_ticks: u64) -> Self {
        self.elapsed_ticks = elapsed_ticks;
        self
    }
}

impl PlatformTimer for ApicTimer {
    fn kind(&self) -> TimerKind {
        TimerKind::Apic
    }

    fn monotonic_now(&self) -> u64 {
        ticks_to_ns(self.elapsed_ticks, self.frequency_hz)
    }

    fn timer_frequency(&self) -> u64 {
        self.frequency_hz
    }
}

/// HPET timer descriptor discovered from firmware tables.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HpetTimer {
    frequency_hz: u64,
    counter: u64,
}

impl HpetTimer {
    pub const fn new(frequency_hz: u64) -> Result<Self, TimerError> {
        if frequency_hz == 0 {
            Err(TimerError::InvalidFrequency)
        } else {
            Ok(Self {
                frequency_hz,
                counter: 0,
            })
        }
    }

    pub const fn with_counter(mut self, counter: u64) -> Self {
        self.counter = counter;
        self
    }
}

impl PlatformTimer for HpetTimer {
    fn kind(&self) -> TimerKind {
        TimerKind::Hpet
    }

    fn monotonic_now(&self) -> u64 {
        ticks_to_ns(self.counter, self.frequency_hz)
    }

    fn timer_frequency(&self) -> u64 {
        self.frequency_hz
    }
}

/// Legacy PIT fallback descriptor. This exists only as a last-resort mechanism.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PitFallbackTimer {
    frequency_hz: u64,
    ticks: u64,
}

impl PitFallbackTimer {
    pub const DEFAULT_FREQUENCY_HZ: u64 = 1_193_182;

    pub const fn new(frequency_hz: u64) -> Result<Self, TimerError> {
        if frequency_hz == 0 {
            Err(TimerError::InvalidFrequency)
        } else {
            Ok(Self {
                frequency_hz,
                ticks: 0,
            })
        }
    }

    pub const fn with_ticks(mut self, ticks: u64) -> Self {
        self.ticks = ticks;
        self
    }
}

impl PlatformTimer for PitFallbackTimer {
    fn kind(&self) -> TimerKind {
        TimerKind::PitFallback
    }

    fn monotonic_now(&self) -> u64 {
        ticks_to_ns(self.ticks, self.frequency_hz)
    }

    fn timer_frequency(&self) -> u64 {
        self.frequency_hz
    }
}

/// Runtime enum for the timer selected during platform discovery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectedPlatformTimer {
    Tsc(TscTimer),
    Apic(ApicTimer),
    Hpet(HpetTimer),
    PitFallback(PitFallbackTimer),
}

impl PlatformTimer for SelectedPlatformTimer {
    fn kind(&self) -> TimerKind {
        match self {
            Self::Tsc(timer) => timer.kind(),
            Self::Apic(timer) => timer.kind(),
            Self::Hpet(timer) => timer.kind(),
            Self::PitFallback(timer) => timer.kind(),
        }
    }

    fn monotonic_now(&self) -> u64 {
        match self {
            Self::Tsc(timer) => timer.monotonic_now(),
            Self::Apic(timer) => timer.monotonic_now(),
            Self::Hpet(timer) => timer.monotonic_now(),
            Self::PitFallback(timer) => timer.monotonic_now(),
        }
    }

    fn timer_frequency(&self) -> u64 {
        match self {
            Self::Tsc(timer) => timer.timer_frequency(),
            Self::Apic(timer) => timer.timer_frequency(),
            Self::Hpet(timer) => timer.timer_frequency(),
            Self::PitFallback(timer) => timer.timer_frequency(),
        }
    }
}

/// Discovery facts supplied by AMD64 and platform probing code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerDiscovery {
    pub cpu_id: AmdCpuId,
    pub ryzen: RyzenPlatform,
    pub tsc_calibration: Option<TscCalibration>,
    pub apic_frequency_hz: Option<u64>,
    pub hpet_frequency_hz: Option<u64>,
    pub pit_frequency_hz: u64,
}

impl TimerDiscovery {
    pub const fn new(cpu_id: AmdCpuId, ryzen: RyzenPlatform) -> Self {
        Self {
            cpu_id,
            ryzen,
            tsc_calibration: None,
            apic_frequency_hz: None,
            hpet_frequency_hz: None,
            pit_frequency_hz: PitFallbackTimer::DEFAULT_FREQUENCY_HZ,
        }
    }

    pub const fn with_tsc_calibration(mut self, calibration: TscCalibration) -> Self {
        self.tsc_calibration = Some(calibration);
        self
    }

    pub const fn with_apic_frequency(mut self, frequency_hz: u64) -> Self {
        self.apic_frequency_hz = Some(frequency_hz);
        self
    }

    pub const fn with_hpet_frequency(mut self, frequency_hz: u64) -> Self {
        self.hpet_frequency_hz = Some(frequency_hz);
        self
    }

    pub const fn with_pit_frequency(mut self, frequency_hz: u64) -> Self {
        self.pit_frequency_hz = frequency_hz;
        self
    }
}

/// Select and calibrate the best available timer mechanism.
pub fn calibrate_timer(discovery: TimerDiscovery) -> Result<SelectedPlatformTimer, TimerError> {
    let features = discovery.cpu_id.features();
    if invariant_tsc_valid(features, discovery.ryzen) {
        if let Some(calibration) = discovery.tsc_calibration {
            return Ok(SelectedPlatformTimer::Tsc(TscTimer::from_calibration(
                calibration,
            )?));
        }
    }

    if features.apic {
        if let Some(frequency_hz) = discovery.apic_frequency_hz {
            return Ok(SelectedPlatformTimer::Apic(ApicTimer::new(frequency_hz)?));
        }
    }

    if let Some(frequency_hz) = discovery.hpet_frequency_hz {
        return Ok(SelectedPlatformTimer::Hpet(HpetTimer::new(frequency_hz)?));
    }

    if discovery.pit_frequency_hz != 0 {
        return Ok(SelectedPlatformTimer::PitFallback(PitFallbackTimer::new(
            discovery.pit_frequency_hz,
        )?));
    }

    Err(TimerError::NoTimerAvailable)
}

/// Read a timer's monotonic nanosecond value.
pub fn monotonic_now(timer: &impl PlatformTimer) -> u64 {
    timer.monotonic_now()
}

/// Read a timer's calibrated frequency in Hz.
pub fn timer_frequency(timer: &impl PlatformTimer) -> u64 {
    timer.timer_frequency()
}

fn invariant_tsc_valid(features: AmdFeatureSet, ryzen: RyzenPlatform) -> bool {
    features.tsc
        && features.invariant_tsc
        && ryzen.supports_feature(RyzenFeatureProfile::InvariantTsc)
            != RyzenSupportStatus::Unsupported
        && (!ryzen.requires_quirk(RyzenQuirk::TscInvariantRequired) || features.invariant_tsc)
}

const fn ticks_to_ns(ticks: u64, frequency_hz: u64) -> u64 {
    if frequency_hz == 0 {
        0
    } else {
        ((ticks as u128 * 1_000_000_000u128) / frequency_hz as u128) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirage_amd64::{AmdCpuidReader, CpuidLeaf};
    use mirage_ryzen::{RyzenCpuId, RyzenDetectionInput, AMD_CPUID_VENDOR};

    struct MockCpuid {
        leaf1_edx: u32,
        ext7_edx: u32,
    }

    impl AmdCpuidReader for MockCpuid {
        fn cpuid(&self, leaf: u32, _subleaf: u32) -> CpuidLeaf {
            match leaf {
                0x0000_0000 => CpuidLeaf::new(
                    0x0000_0001,
                    u32::from_le_bytes(*b"Auth"),
                    u32::from_le_bytes(*b"cAMD"),
                    u32::from_le_bytes(*b"enti"),
                ),
                0x0000_0001 => CpuidLeaf::new(0x0080_0f82, 0, 0, self.leaf1_edx),
                0x8000_0000 => CpuidLeaf::new(0x8000_0007, 0, 0, 0),
                0x8000_0007 => CpuidLeaf::new(0, 0, 0, self.ext7_edx),
                _ => CpuidLeaf::new(0, 0, 0, 0),
            }
        }
    }

    struct MockTsc {
        values: &'static [u64],
        index: usize,
    }

    impl TscCounter for MockTsc {
        fn read_tsc(&mut self) -> u64 {
            let value = self.values[self.index.min(self.values.len() - 1)];
            self.index += 1;
            value
        }
    }

    struct MockReference {
        values: &'static [u64],
        index: usize,
    }

    impl ReferenceTimer for MockReference {
        fn now_ns(&mut self) -> u64 {
            let value = self.values[self.index.min(self.values.len() - 1)];
            self.index += 1;
            value
        }
    }

    fn cpu(tsc: bool, apic: bool, invariant: bool) -> AmdCpuId {
        let mut leaf1_edx = 0;
        if tsc {
            leaf1_edx |= 1 << 4;
        }
        if apic {
            leaf1_edx |= 1 << 9;
        }
        let ext7_edx = if invariant { 1 << 8 } else { 0 };
        AmdCpuId::from_reader(&MockCpuid {
            leaf1_edx,
            ext7_edx,
        })
    }

    fn ryzen() -> RyzenPlatform {
        RyzenPlatform::from_detection_input(RyzenDetectionInput::new(
            AMD_CPUID_VENDOR,
            RyzenCpuId::new(0x17, 0x08, 2),
            None,
        ))
    }

    #[test]
    fn mock_tsc_calibration_computes_frequency_and_monotonic_time() {
        let timer = TscTimer::from_calibration(TscCalibration::new(1_000, 3_000, 1_000)).unwrap();
        assert_eq!(timer.timer_frequency(), 2_000_000_000);
        assert_eq!(timer.with_current_tsc(4_000).monotonic_now(), 500);
    }

    #[test]
    fn selection_follows_tsc_apic_hpet_pit_priority() {
        let selected = calibrate_timer(
            TimerDiscovery::new(cpu(true, true, true), ryzen())
                .with_tsc_calibration(TscCalibration::new(0, 10, 10))
                .with_apic_frequency(1_000_000)
                .with_hpet_frequency(14_318_180),
        )
        .unwrap();
        assert_eq!(selected.kind(), TimerKind::Tsc);

        let selected = calibrate_timer(
            TimerDiscovery::new(cpu(false, true, false), ryzen())
                .with_apic_frequency(1_000_000)
                .with_hpet_frequency(14_318_180),
        )
        .unwrap();
        assert_eq!(selected.kind(), TimerKind::Apic);

        let selected = calibrate_timer(
            TimerDiscovery::new(cpu(false, false, false), ryzen()).with_hpet_frequency(14_318_180),
        )
        .unwrap();
        assert_eq!(selected.kind(), TimerKind::Hpet);

        let selected =
            calibrate_timer(TimerDiscovery::new(cpu(false, false, false), ryzen())).unwrap();
        assert_eq!(selected.kind(), TimerKind::PitFallback);
    }

    #[test]
    fn invalid_tsc_falls_back_to_apic_timer() {
        let selected = calibrate_timer(
            TimerDiscovery::new(cpu(true, true, false), ryzen())
                .with_tsc_calibration(TscCalibration::new(0, 10, 10))
                .with_apic_frequency(1_000_000),
        )
        .unwrap();
        assert_eq!(selected.kind(), TimerKind::Apic);
    }

    #[test]
    fn polling_calibration_times_out_when_reference_does_not_advance() {
        let mut tsc = MockTsc {
            values: &[100, 200],
            index: 0,
        };
        let mut reference = MockReference {
            values: &[5, 5, 5, 5],
            index: 0,
        };
        assert_eq!(
            TscTimer::calibrate_with_polling(&mut tsc, &mut reference, 10, 3),
            Err(TimerError::CalibrationTimeout)
        );
    }
}
