#![no_std]
#![cfg_attr(


    not(all(feature = "hw-ryzen", target_arch = "x86_64")),
    forbid(unsafe_code)
)]
#![deny(unsafe_op_in_unsafe_fn)]

//! Ryzen-specific mechanism descriptors.
//!
//! This crate models CPU/package facts that the supervisor can use to make
//! policy decisions elsewhere. It does not choose drivers or grant authority.
//! Detection is intentionally bucketed by CPUID family/model/stepping and, when
//! a caller has already discovered them, optional PCI IDs. It avoids
//! marketing-name claims and falls back to structured unknown status instead of
//! panicking on unmodeled AMD64 processors.

use mirage_amd64::{
    AmdCacheInfo, AmdCpuId, AmdFeatureSet, AmdTopology, AmdVendor, PrivilegeRing,
};

pub mod renoir;

pub use renoir::{RenoirCpuProfile, RenoirDetectionKind};

/// Fixed-size CPUID brand string buffer. Bytes after the first NUL are padding.
pub type RyzenBrandString = [u8; 48];

/// Hardware-backed AMD Ryzen/APU platform information surfaced during boot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RyzenPlatformInfo {
    pub vendor: [u8; 12],
    pub family: u16,
    pub model: u16,
    pub stepping: u8,
    pub brand_string: RyzenBrandString,
    pub core_count: u16,
    pub thread_count: u16,
    pub has_invariant_tsc: bool,
    pub has_x2apic: bool,
    pub has_sme: bool,
    pub has_svm: bool,
}

impl RyzenPlatformInfo {
    pub fn discover() -> Self {
        Self::from_cpuid(AmdCpuId::read())
    }

    pub fn from_cpuid(cpuid: AmdCpuId) -> Self {
        let (family, model, stepping) = cpuid.family_model_stepping();
        let features = cpuid.features();
        let topology = AmdTopology::from_cpuid(cpuid);
        Self::from_parts(
            cpuid.vendor().as_bytes(),
            family.0,
            model.0,
            stepping.0,
            cpuid.brand_string(),
            topology.cores_per_package,
            topology.logical_processors_per_package,
            features,
        )
    }

    pub const fn from_parts(
        vendor: [u8; 12],
        family: u16,
        model: u16,
        stepping: u8,
        brand_string: RyzenBrandString,
        core_count: u16,
        thread_count: u16,
        features: AmdFeatureSet,
    ) -> Self {
        Self {
            vendor,
            family,
            model,
            stepping,
            brand_string,
            core_count,
            thread_count,
            has_invariant_tsc: features.invariant_tsc,
            has_x2apic: features.x2apic,
            has_sme: features.sme,
            has_svm: features.svm,
        }
    }

    pub const fn is_amd(self) -> bool {
        matches!(AmdVendor::from_bytes(self.vendor), AmdVendor::Amd)
    }

    pub const fn is_renoir_lucienne_zen2_mobile(self) -> bool {
        self.is_amd() && self.family == 0x17 && self.model >= 0x60 && self.model <= 0x7f
    }

    pub const fn is_ryzen_4500u_class(self) -> bool {
        self.is_renoir_lucienne_zen2_mobile() && self.core_count == 6 && self.thread_count == 6
    }
}

/// One scheduler-visible logical CPU placement record.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RyzenTopologyEntry {
    pub logical_cpu_id: u16,
    pub package_id: u16,
    pub physical_core_id: u16,
    pub smt_sibling_id: Option<u16>,
    pub cache_group_id: u16,
    pub preferred_core: bool,
}

/// Fixed no-heap topology table for early Ryzen boot reporting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RyzenTopologyTable {
    entries: [Option<RyzenTopologyEntry>; Self::MAX_LOGICAL_CPUS],
    len: usize,
    pub cache: AmdCacheInfo,
}

impl RyzenTopologyTable {
    pub const MAX_LOGICAL_CPUS: usize = 256;

    pub const fn empty(cache: AmdCacheInfo) -> Self {
        Self {
            entries: [None; Self::MAX_LOGICAL_CPUS],
            len: 0,
            cache,
        }
    }

    pub fn from_current_cpu(cpuid: AmdCpuId) -> Self {
        let topology = AmdTopology::from_cpuid(cpuid);
        let cache = AmdCacheInfo::from_cpuid(cpuid);
        let mut table = Self::empty(cache);
        let count = topology
            .logical_processors_per_package
            .min(Self::MAX_LOGICAL_CPUS as u16);
        let threads_per_core = topology.threads_per_core.max(1);
        let mut cpu = 0u16;
        while cpu < count {
            let core = cpu / threads_per_core;
            let sibling = if threads_per_core > 1 {
                Some(core * threads_per_core + ((cpu + 1) % threads_per_core))
            } else {
                None
            };
            table.entries[cpu as usize] = Some(RyzenTopologyEntry {
                logical_cpu_id: cpu,
                package_id: topology.package_id.0,
                physical_core_id: core,
                smt_sibling_id: sibling,
                cache_group_id: core,
                preferred_core: false,
            });
            cpu += 1;
        }
        table.len = count as usize;
        table
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub fn entries(&self) -> &[Option<RyzenTopologyEntry>] {
        &self.entries[..self.len]
    }
}

pub fn mirage_ryzen_topology() -> RyzenTopologyTable {
    RyzenTopologyTable::from_current_cpu(AmdCpuId::read())
}

/// The CPUID vendor string used by AMD processors.
pub const AMD_CPUID_VENDOR: [u8; 12] = *b"AuthenticAMD";

/// AMD CPU family/model/stepping tuple decoded by architecture probing code.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RyzenCpuId {
    family: u16,
    model: u16,
    stepping: u8,
}

impl RyzenCpuId {
    pub const fn new(family: u16, model: u16, stepping: u8) -> Self {
        Self {
            family,
            model,
            stepping,
        }
    }

    pub const fn family(self) -> u16 {
        self.family
    }

    pub const fn model(self) -> u16 {
        self.model
    }

    pub const fn stepping(self) -> u8 {
        self.stepping
    }

    /// Decode the effective AMD64 CPUID family/model/stepping tuple from CPUID
    /// leaf 1 EAX.
    pub const fn from_leaf1_eax(eax: u32) -> Self {
        let base_family = ((eax >> 8) & 0x0f) as u16;
        let base_model = ((eax >> 4) & 0x0f) as u16;
        let stepping = (eax & 0x0f) as u8;
        let extended_family = ((eax >> 20) & 0xff) as u16;
        let extended_model = ((eax >> 16) & 0x0f) as u16;

        let family = if base_family == 0x0f {
            base_family + extended_family
        } else {
            base_family
        };
        let model = if base_family == 0x0f || base_family == 0x06 {
            base_model + (extended_model << 4)
        } else {
            base_model
        };

        Self::new(family, model, stepping)
    }
}

/// Topology facts surfaced by low-level discovery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RyzenTopology {
    pub packages: u16,
    pub cores_per_package: u16,
    pub threads_per_core: u16,
}

impl RyzenTopology {
    pub const fn new(packages: u16, cores_per_package: u16, threads_per_core: u16) -> Self {
        Self {
            packages,
            cores_per_package,
            threads_per_core,
        }
    }

    pub const fn logical_cpus(self) -> u32 {
        self.packages as u32 * self.cores_per_package as u32 * self.threads_per_core as u32
    }
}

/// Telemetry channel identifiers exposed as mechanism, not policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RyzenTelemetryChannel {
    TemperatureCelsius,
    PackagePowerMilliwatts,
    CoreVoltageMillivolts,
}

/// A low-level Ryzen hardware profile discovered before supervisor policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RyzenProfile {
    pub cpu_id: RyzenCpuId,
    pub topology: RyzenTopology,
    pub required_ring: PrivilegeRing,
}

impl RyzenProfile {
    pub const fn new(cpu_id: RyzenCpuId, topology: RyzenTopology) -> Self {
        Self {
            cpu_id,
            topology,
            required_ring: PrivilegeRing::Ring0,
        }
    }
}

/// Bucketed AMD64 generation used by Mirage for mechanism selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RyzenGeneration {
    Zen,
    ZenPlus,
    Zen2,
    Zen3,
    Zen4,
    Zen5,
    UnknownAmd64,
}

/// AMD64 CPU family wrapper. The raw value is preserved for diagnostics even
/// when Mirage does not recognize the processor generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RyzenFamily(u16);

impl RyzenFamily {
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u16 {
        self.0
    }
}

/// AMD64 CPU model wrapper. The raw value is preserved for diagnostics even
/// when Mirage does not recognize the processor generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RyzenModel(u16);

impl RyzenModel {
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u16 {
        self.0
    }
}

/// Optional SoC classification derived from non-authoritative PCI hints.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RyzenSocKind {
    Desktop,
    Mobile,
    Server,
    Embedded,
    Unknown,
}

/// Feature probes surfaced as structured support state, not policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RyzenFeatureProfile {
    Amd64LongMode,
    InvariantTsc,
    X2Apic,
    IommuIsolation,
    PciIdAssistedSocDetection,
}

/// Structured feature/detection support result.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RyzenSupportStatus {
    Supported,
    Unsupported,
    Unknown,
}

/// Detection status for the platform descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RyzenDetectionStatus {
    Detected,
    UnknownAmd64,
    UnsupportedVendor,
    HardwareProbeUnavailable,
}

/// PCI device ID hint supplied by platform discovery code when available.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RyzenPciId {
    pub vendor_id: u16,
    pub device_id: u16,
}

impl RyzenPciId {
    pub const AMD_VENDOR_ID: u16 = 0x1022;

    pub const fn new(vendor_id: u16, device_id: u16) -> Self {
        Self {
            vendor_id,
            device_id,
        }
    }
}

/// Input supplied by CPUID and optional PCI discovery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RyzenDetectionInput {
    pub vendor: [u8; 12],
    pub cpu_id: RyzenCpuId,
    pub pci_id: Option<RyzenPciId>,
}

impl RyzenDetectionInput {
    pub const fn new(vendor: [u8; 12], cpu_id: RyzenCpuId, pci_id: Option<RyzenPciId>) -> Self {
        Self {
            vendor,
            cpu_id,
            pci_id,
        }
    }

    pub const fn amd(cpu_id: RyzenCpuId) -> Self {
        Self::new(AMD_CPUID_VENDOR, cpu_id, None)
    }
}

/// Ryzen quirk identifiers. These are mechanism descriptors consumed by
/// supervisor/platform policy; the kernel does not decide how to respond.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RyzenQuirk {
    TscInvariantRequired,
    PreferX2Apic,
    DisableBrokenFeature,
    RequiresIommuIsolation,
    TimerCalibrationNeeded,
}

/// A single quirk scaffold entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RyzenQuirkEntry {
    pub generation: RyzenGeneration,
    pub soc_kind: Option<RyzenSocKind>,
    pub quirk: RyzenQuirk,
}

/// Static quirk table used by platform code for structured lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RyzenQuirkTable {
    entries: &'static [RyzenQuirkEntry],
}

impl RyzenQuirkTable {
    pub const fn new(entries: &'static [RyzenQuirkEntry]) -> Self {
        Self { entries }
    }

    pub const fn default() -> Self {
        Self::new(DEFAULT_RYZEN_QUIRKS)
    }

    pub const fn entries(self) -> &'static [RyzenQuirkEntry] {
        self.entries
    }

    pub fn requires(
        self,
        generation: RyzenGeneration,
        soc_kind: RyzenSocKind,
        quirk: RyzenQuirk,
    ) -> bool {
        self.entries.iter().any(|entry| {
            entry.generation == generation
                && entry.quirk == quirk
                && match entry.soc_kind {
                    Some(required_soc) => required_soc == soc_kind,
                    None => true,
                }
        })
    }
}

/// Scaffold quirk entries. They are deliberately conservative and generic:
/// concrete policy remains in the supervisor.
pub const DEFAULT_RYZEN_QUIRKS: &[RyzenQuirkEntry] = &[
    RyzenQuirkEntry {
        generation: RyzenGeneration::Zen,
        soc_kind: None,
        quirk: RyzenQuirk::TscInvariantRequired,
    },
    RyzenQuirkEntry {
        generation: RyzenGeneration::ZenPlus,
        soc_kind: None,
        quirk: RyzenQuirk::PreferX2Apic,
    },
    RyzenQuirkEntry {
        generation: RyzenGeneration::Zen2,
        soc_kind: None,
        quirk: RyzenQuirk::DisableBrokenFeature,
    },
    RyzenQuirkEntry {
        generation: RyzenGeneration::Zen3,
        soc_kind: Some(RyzenSocKind::Server),
        quirk: RyzenQuirk::RequiresIommuIsolation,
    },
    RyzenQuirkEntry {
        generation: RyzenGeneration::UnknownAmd64,
        soc_kind: None,
        quirk: RyzenQuirk::TimerCalibrationNeeded,
    },
];

/// AMD telemetry scaffold types and mock readers.
///
/// This module is intentionally read-only: it models telemetry facts for a
/// future supervised service, but it does not expose tuning, overclocking,
/// firmware mutation, SMU writes, or permanent firmware changes. Any future
/// real hardware collector must remain behind the `hw-amd-telemetry` feature
/// and receive narrowly scoped supervisor capabilities.
pub mod telemetry {
    use super::{
        RyzenDetectionStatus, RyzenGeneration, RyzenPlatform, RyzenSupportStatus,
        RyzenTelemetryChannel,
    };

    /// Errors returned by AMD telemetry discovery and read scaffolds.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub enum AmdTelemetryError {
        /// The current CPU vendor is not AMD.
        UnsupportedVendor,
        /// Mirage cannot prove that the requested telemetry channel exists.
        SensorUnavailable,
        /// AMD P-state reporting is not known to be available for this CPU.
        PstateUnsupported,
        /// Real hardware telemetry was requested without enabling the gated path.
        HardwarePathDisabled,
        /// A caller attempted to request a non-telemetry control operation.
        UnsafeOperationRejected,
    }

    /// Read-only AMD thermal sensor sample.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct AmdThermalSensor {
        pub channel: RyzenTelemetryChannel,
        pub temperature_millicelsius: i32,
        pub fresh: bool,
    }

    impl AmdThermalSensor {
        pub const fn new(
            channel: RyzenTelemetryChannel,
            temperature_millicelsius: i32,
            fresh: bool,
        ) -> Self {
            Self {
                channel,
                temperature_millicelsius,
                fresh,
            }
        }

        pub const fn temperature_celsius(self) -> i32 {
            self.temperature_millicelsius / 1_000
        }
    }

    /// Read-only AMD power-state sample.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct AmdPowerState {
        pub pstate_id: u8,
        pub effective_frequency_mhz: u32,
        pub package_power_milliwatts: u32,
        pub boost_active: bool,
    }

    impl AmdPowerState {
        pub const fn new(
            pstate_id: u8,
            effective_frequency_mhz: u32,
            package_power_milliwatts: u32,
            boost_active: bool,
        ) -> Self {
            Self {
                pstate_id,
                effective_frequency_mhz,
                package_power_milliwatts,
                boost_active,
            }
        }
    }

    /// Structured AMD P-state support information.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct AmdPstateInfo {
        pub support: RyzenSupportStatus,
        pub generation: RyzenGeneration,
        pub requires_supervisor_service: bool,
        pub reason: &'static str,
    }

    impl AmdPstateInfo {
        pub const fn new(
            support: RyzenSupportStatus,
            generation: RyzenGeneration,
            requires_supervisor_service: bool,
            reason: &'static str,
        ) -> Self {
            Self {
                support,
                generation,
                requires_supervisor_service,
                reason,
            }
        }

        pub const fn is_supported(self) -> bool {
            matches!(self.support, RyzenSupportStatus::Supported)
        }
    }

    /// Combined AMD telemetry snapshot for mock service integration tests.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct AmdTelemetry {
        pub temperature: AmdThermalSensor,
        pub power_state: AmdPowerState,
        pub pstate: AmdPstateInfo,
    }

    impl AmdTelemetry {
        pub fn read_mock(platform: RyzenPlatform) -> Result<Self, AmdTelemetryError> {
            if platform.detection_status() == RyzenDetectionStatus::UnsupportedVendor {
                return Err(AmdTelemetryError::UnsupportedVendor);
            }

            Ok(Self {
                temperature: read_temperature_mock()?,
                power_state: read_power_state_mock()?,
                pstate: detect_pstate_support(platform),
            })
        }
    }

    /// Return a deterministic mock thermal sensor reading.
    pub const fn read_temperature_mock() -> Result<AmdThermalSensor, AmdTelemetryError> {
        Ok(AmdThermalSensor::new(
            RyzenTelemetryChannel::TemperatureCelsius,
            42_000,
            true,
        ))
    }

    /// Return a deterministic mock power-state reading.
    pub const fn read_power_state_mock() -> Result<AmdPowerState, AmdTelemetryError> {
        Ok(AmdPowerState::new(1, 3_600, 45_000, false))
    }

    /// Detect AMD P-state support from already-discovered mechanism facts.
    ///
    /// This is a conservative scaffold, not a hardware driver. It does not read
    /// or write SMU/MSR state, and it does not tune voltage, frequency, power
    /// limits, firmware settings, or boost behavior.
    pub const fn detect_pstate_support(platform: RyzenPlatform) -> AmdPstateInfo {
        match platform.detection_status() {
            RyzenDetectionStatus::UnsupportedVendor => AmdPstateInfo::new(
                RyzenSupportStatus::Unsupported,
                platform.generation(),
                false,
                "non-AMD vendor",
            ),
            RyzenDetectionStatus::HardwareProbeUnavailable => AmdPstateInfo::new(
                RyzenSupportStatus::Unknown,
                platform.generation(),
                true,
                "hardware probe unavailable in this build",
            ),
            RyzenDetectionStatus::UnknownAmd64 => AmdPstateInfo::new(
                RyzenSupportStatus::Unknown,
                platform.generation(),
                true,
                "unmodeled AMD64 family/model",
            ),
            RyzenDetectionStatus::Detected => match platform.generation() {
                RyzenGeneration::Zen | RyzenGeneration::ZenPlus => AmdPstateInfo::new(
                    RyzenSupportStatus::Unknown,
                    platform.generation(),
                    true,
                    "early Zen generation requires PPR-specific confirmation",
                ),
                RyzenGeneration::Zen2
                | RyzenGeneration::Zen3
                | RyzenGeneration::Zen4
                | RyzenGeneration::Zen5 => AmdPstateInfo::new(
                    RyzenSupportStatus::Supported,
                    platform.generation(),
                    true,
                    "supported by Mirage telemetry scaffold",
                ),
                RyzenGeneration::UnknownAmd64 => AmdPstateInfo::new(
                    RyzenSupportStatus::Unknown,
                    platform.generation(),
                    true,
                    "unmodeled AMD64 generation",
                ),
            },
        }
    }

    /// Placeholder for future read-only hardware telemetry.
    #[cfg(feature = "hw-amd-telemetry")]
    pub fn read_hardware_snapshot(
        _platform: RyzenPlatform,
    ) -> Result<AmdTelemetry, AmdTelemetryError> {
        Err(AmdTelemetryError::HardwarePathDisabled)
    }

    /// Hardware telemetry is intentionally unavailable unless explicitly built.
    #[cfg(not(feature = "hw-amd-telemetry"))]
    pub fn read_hardware_snapshot(
        _platform: RyzenPlatform,
    ) -> Result<AmdTelemetry, AmdTelemetryError> {
        Err(AmdTelemetryError::HardwarePathDisabled)
    }
}

pub use telemetry::{
    detect_pstate_support, read_power_state_mock, read_temperature_mock, AmdPowerState,
    AmdPstateInfo, AmdTelemetry, AmdTelemetryError, AmdThermalSensor,
};

/// Bucketed Ryzen platform descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RyzenPlatform {
    vendor: [u8; 12],
    family: RyzenFamily,
    model: RyzenModel,
    stepping: u8,
    generation: RyzenGeneration,
    soc_kind: RyzenSocKind,
    status: RyzenDetectionStatus,
    quirk_table: RyzenQuirkTable,
}

impl RyzenPlatform {
    /// Detect the current processor where hardware probing is available. Mock
    /// builds degrade to `UnknownAmd64` rather than manufacturing host facts.
    pub fn detect() -> Self {
        match detect_input() {
            Some(input) => Self::from_detection_input(input),
            None => Self::unknown_with_status(RyzenDetectionStatus::HardwareProbeUnavailable),
        }
    }

    /// Build a platform descriptor from mocked or previously discovered CPUID
    /// and optional PCI facts.
    pub const fn from_detection_input(input: RyzenDetectionInput) -> Self {
        let generation = classify_generation(input.vendor, input.cpu_id);
        let status = classify_status(input.vendor, generation);
        Self {
            vendor: input.vendor,
            family: RyzenFamily::new(input.cpu_id.family()),
            model: RyzenModel::new(input.cpu_id.model()),
            stepping: input.cpu_id.stepping(),
            generation,
            soc_kind: classify_soc(input.pci_id),
            status,
            quirk_table: RyzenQuirkTable::default(),
        }
    }

    pub const fn unknown_with_status(status: RyzenDetectionStatus) -> Self {
        Self {
            vendor: AMD_CPUID_VENDOR,
            family: RyzenFamily::new(0),
            model: RyzenModel::new(0),
            stepping: 0,
            generation: RyzenGeneration::UnknownAmd64,
            soc_kind: RyzenSocKind::Unknown,
            status,
            quirk_table: RyzenQuirkTable::default(),
        }
    }

    pub const fn generation(self) -> RyzenGeneration {
        self.generation
    }

    pub const fn family(self) -> RyzenFamily {
        self.family
    }

    pub const fn model(self) -> RyzenModel {
        self.model
    }

    pub const fn stepping(self) -> u8 {
        self.stepping
    }

    pub const fn soc_kind(self) -> RyzenSocKind {
        self.soc_kind
    }

    pub const fn detection_status(self) -> RyzenDetectionStatus {
        self.status
    }

    pub const fn vendor(self) -> [u8; 12] {
        self.vendor
    }

    pub fn supports_feature(self, feature: RyzenFeatureProfile) -> RyzenSupportStatus {
        if self.status == RyzenDetectionStatus::UnsupportedVendor {
            return RyzenSupportStatus::Unsupported;
        }

        match (self.generation, feature) {
            (_, RyzenFeatureProfile::Amd64LongMode) => RyzenSupportStatus::Supported,
            (RyzenGeneration::UnknownAmd64, _) => RyzenSupportStatus::Unknown,
            (_, RyzenFeatureProfile::InvariantTsc) => RyzenSupportStatus::Supported,
            (RyzenGeneration::Zen, RyzenFeatureProfile::X2Apic) => RyzenSupportStatus::Unknown,
            (_, RyzenFeatureProfile::X2Apic) => RyzenSupportStatus::Supported,
            (_, RyzenFeatureProfile::IommuIsolation) => RyzenSupportStatus::Unknown,
            (_, RyzenFeatureProfile::PciIdAssistedSocDetection) => {
                if self.soc_kind == RyzenSocKind::Unknown {
                    RyzenSupportStatus::Unknown
                } else {
                    RyzenSupportStatus::Supported
                }
            }
        }
    }

    pub fn requires_quirk(self, quirk: RyzenQuirk) -> bool {
        self.quirk_table
            .requires(self.generation, self.soc_kind, quirk)
    }
}

const fn classify_status(vendor: [u8; 12], generation: RyzenGeneration) -> RyzenDetectionStatus {
    if !is_amd_vendor(vendor) {
        RyzenDetectionStatus::UnsupportedVendor
    } else if matches!(generation, RyzenGeneration::UnknownAmd64) {
        RyzenDetectionStatus::UnknownAmd64
    } else {
        RyzenDetectionStatus::Detected
    }
}

const fn classify_generation(vendor: [u8; 12], cpu_id: RyzenCpuId) -> RyzenGeneration {
    if !is_amd_vendor(vendor) {
        return RyzenGeneration::UnknownAmd64;
    }

    match cpu_id.family() {
        0x17 => match cpu_id.model() {
            0x00..=0x0f => RyzenGeneration::Zen,
            0x10..=0x2f => RyzenGeneration::ZenPlus,
            0x30..=0x7f => RyzenGeneration::Zen2,
            _ => RyzenGeneration::UnknownAmd64,
        },
        0x19 => match cpu_id.model() {
            0x00..=0x2f => RyzenGeneration::Zen3,
            0x30..=0x7f => RyzenGeneration::Zen4,
            _ => RyzenGeneration::UnknownAmd64,
        },
        0x1a => RyzenGeneration::Zen5,
        _ => RyzenGeneration::UnknownAmd64,
    }
}

const fn classify_soc(pci_id: Option<RyzenPciId>) -> RyzenSocKind {
    match pci_id {
        Some(id) if id.vendor_id == RyzenPciId::AMD_VENDOR_ID => match id.device_id {
            0x1450..=0x147f => RyzenSocKind::Desktop,
            0x1480..=0x149f | 0x14a0..=0x14bf => RyzenSocKind::Server,
            0x15d0..=0x15ff => RyzenSocKind::Mobile,
            0x1630..=0x163f => RyzenSocKind::Embedded,
            _ => RyzenSocKind::Unknown,
        },
        _ => RyzenSocKind::Unknown,
    }
}

const fn is_amd_vendor(vendor: [u8; 12]) -> bool {
    let mut index = 0;
    while index < AMD_CPUID_VENDOR.len() {
        if vendor[index] != AMD_CPUID_VENDOR[index] {
            return false;
        }
        index += 1;
    }
    true
}

#[cfg(all(feature = "hw-ryzen", target_arch = "x86_64"))]
fn detect_input() -> Option<RyzenDetectionInput> {
    use core::arch::x86_64::__cpuid;

    // SAFETY: CPUID is a CPU identification instruction available on this x86_64 backend.
    let vendor_leaf = unsafe { __cpuid(0) };
    let mut vendor = [0_u8; 12];
    vendor[0..4].copy_from_slice(&vendor_leaf.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&vendor_leaf.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&vendor_leaf.ecx.to_le_bytes());

    // SAFETY: Same CPUID backend as above; leaf 1 is architectural on x86_64.
    let leaf1 = unsafe { __cpuid(1) };
    Some(RyzenDetectionInput::new(
        vendor,
        RyzenCpuId::from_leaf1_eax(leaf1.eax),
        None,
    ))
}

#[cfg(any(not(feature = "hw-ryzen"), not(target_arch = "x86_64")))]
fn detect_input() -> Option<RyzenDetectionInput> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct MockCpuid<'a> {
        leaves: &'a [(u32, u32, mirage_amd64::CpuidLeaf)],
    }
    impl mirage_amd64::AmdCpuidReader for MockCpuid<'_> {
        fn cpuid(&self, leaf: u32, subleaf: u32) -> mirage_amd64::CpuidLeaf {
            self.leaves
                .iter()
                .find(|(l, s, _)| *l == leaf && *s == subleaf)
                .map(|(_, _, v)| *v)
                .unwrap_or_default()
        }
    }
    fn vendor_leaf() -> mirage_amd64::CpuidLeaf {
        mirage_amd64::CpuidLeaf::new(
            0x0000_0001,
            u32::from_le_bytes(*b"Auth"),
            u32::from_le_bytes(*b"cAMD"),
            u32::from_le_bytes(*b"enti"),
        )
    }

    fn platform(family: u16, model: u16) -> RyzenPlatform {
        RyzenPlatform::from_detection_input(RyzenDetectionInput::amd(RyzenCpuId::new(
            family, model, 1,
        )))
    }

    #[test]
    fn detects_mock_zen_through_zen5_buckets() {
        assert_eq!(platform(0x17, 0x01).generation(), RyzenGeneration::Zen);
        assert_eq!(platform(0x17, 0x18).generation(), RyzenGeneration::ZenPlus);
        assert_eq!(platform(0x17, 0x31).generation(), RyzenGeneration::Zen2);
        assert_eq!(platform(0x19, 0x01).generation(), RyzenGeneration::Zen3);
        assert_eq!(platform(0x19, 0x61).generation(), RyzenGeneration::Zen4);
        assert_eq!(platform(0x1a, 0x02).generation(), RyzenGeneration::Zen5);
    }

    #[test]
    fn unknown_amd_falls_back_without_panicking() {
        let detected = platform(0x1f, 0xff);

        assert_eq!(detected.generation(), RyzenGeneration::UnknownAmd64);
        assert_eq!(
            detected.detection_status(),
            RyzenDetectionStatus::UnknownAmd64
        );
        assert_eq!(
            detected.supports_feature(RyzenFeatureProfile::InvariantTsc),
            RyzenSupportStatus::Unknown
        );
        assert!(detected.requires_quirk(RyzenQuirk::TimerCalibrationNeeded));
    }

    #[test]
    fn unsupported_vendor_is_structured() {
        let detected = RyzenPlatform::from_detection_input(RyzenDetectionInput::new(
            *b"GenuineIntel",
            RyzenCpuId::new(0x17, 0x01, 0),
            None,
        ));

        assert_eq!(detected.generation(), RyzenGeneration::UnknownAmd64);
        assert_eq!(
            detected.detection_status(),
            RyzenDetectionStatus::UnsupportedVendor
        );
        assert_eq!(
            detected.supports_feature(RyzenFeatureProfile::Amd64LongMode),
            RyzenSupportStatus::Unsupported
        );
    }

    #[test]
    fn quirk_lookup_matches_generation_and_optional_soc() {
        let zen = platform(0x17, 0x01);
        assert!(zen.requires_quirk(RyzenQuirk::TscInvariantRequired));
        assert!(!zen.requires_quirk(RyzenQuirk::RequiresIommuIsolation));

        let server_zen3 = RyzenPlatform::from_detection_input(RyzenDetectionInput::new(
            AMD_CPUID_VENDOR,
            RyzenCpuId::new(0x19, 0x01, 0),
            Some(RyzenPciId::new(RyzenPciId::AMD_VENDOR_ID, 0x1485)),
        ));
        assert_eq!(server_zen3.soc_kind(), RyzenSocKind::Server);
        assert!(server_zen3.requires_quirk(RyzenQuirk::RequiresIommuIsolation));
    }

    #[test]
    fn decodes_effective_cpuid_leaf1_tuple() {
        let cpu_id = RyzenCpuId::from_leaf1_eax((0x8 << 20) | (0x3 << 16) | (0xf << 8) | 1);

        assert_eq!(cpu_id.family(), 0x17);
        assert_eq!(cpu_id.model(), 0x30);
        assert_eq!(cpu_id.stepping(), 1);
    }

    #[test]
    fn mock_telemetry_readings_are_deterministic_and_read_only() {
        let temperature = read_temperature_mock().expect("mock temperature");
        assert_eq!(
            temperature.channel,
            RyzenTelemetryChannel::TemperatureCelsius
        );
        assert_eq!(temperature.temperature_millicelsius, 42_000);
        assert!(temperature.fresh);

        let power = read_power_state_mock().expect("mock power state");
        assert_eq!(power.pstate_id, 1);
        assert_eq!(power.effective_frequency_mhz, 3_600);
        assert_eq!(power.package_power_milliwatts, 45_000);
        assert!(!power.boost_active);
    }

    #[test]
    fn pstate_support_is_structured_by_detected_generation() {
        let zen = detect_pstate_support(platform(0x17, 0x01));
        assert_eq!(zen.support, RyzenSupportStatus::Unknown);
        assert!(zen.requires_supervisor_service);

        let zen4 = detect_pstate_support(platform(0x19, 0x61));
        assert_eq!(zen4.support, RyzenSupportStatus::Supported);
        assert_eq!(zen4.generation, RyzenGeneration::Zen4);
        assert!(zen4.is_supported());
    }

    #[test]
    fn platform_info_identifies_4500u_class_renoir() {
        let mut brand = [0u8; 48];
        brand[..17].copy_from_slice(b"AMD Ryzen 5 4500U");
        let features = mirage_amd64::AmdFeatureSet {
            invariant_tsc: true,
            x2apic: true,
            svm: true,
            sme: true,
            ..mirage_amd64::AmdFeatureSet::default()
        };
        let info =
            RyzenPlatformInfo::from_parts(AMD_CPUID_VENDOR, 0x17, 0x60, 1, brand, 6, 6, features);
        assert!(info.is_renoir_lucienne_zen2_mobile());
        assert!(info.is_ryzen_4500u_class());
        assert!(info.has_invariant_tsc);
        assert!(info.has_x2apic);
        assert!(info.has_sme);
        assert!(info.has_svm);
    }

    #[test]
    fn topology_table_exports_mtss_scheduler_hints() {
        let cpuid = mirage_amd64::AmdCpuId::from_reader(&MockCpuid {
            leaves: &[
                (0x0000_0000, 0, vendor_leaf()),
                (
                    0x0000_0001,
                    0,
                    mirage_amd64::CpuidLeaf::new(0, 6 << 16, 0, 0),
                ),
                (
                    0x8000_0000,
                    0,
                    mirage_amd64::CpuidLeaf::new(0x8000_0008, 0, 0, 0),
                ),
                (0x8000_0008, 0, mirage_amd64::CpuidLeaf::new(0, 0, 5, 0)),
            ],
        });
        let table = RyzenTopologyTable::from_current_cpu(cpuid);

        assert_eq!(table.len(), 6);
        assert_eq!(table.entries()[5].unwrap().physical_core_id, 5);
        assert_eq!(table.entries()[5].unwrap().smt_sibling_id, None);
    }

    #[test]
    fn telemetry_snapshot_rejects_unsupported_vendor() {
        let platform = RyzenPlatform::from_detection_input(RyzenDetectionInput::new(
            *b"GenuineIntel",
            RyzenCpuId::new(0x17, 0x01, 0),
            None,
        ));

        assert_eq!(
            AmdTelemetry::read_mock(platform),
            Err(AmdTelemetryError::UnsupportedVendor)
        );
    }
}
