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

use mirage_amd64::PrivilegeRing;

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

    // SAFETY: CPUID is a serializing userspace-safe x86_64 instruction. This
    // path only reads architectural CPU identification leaves and does not
    // grant hardware authority or make policy decisions.
    let vendor_leaf = unsafe { __cpuid(0) };
    let mut vendor = [0_u8; 12];
    vendor[0..4].copy_from_slice(&vendor_leaf.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&vendor_leaf.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&vendor_leaf.ecx.to_le_bytes());

    // SAFETY: See the safety note above. Leaf 1 is the architectural processor
    // signature leaf used for family/model/stepping decoding.
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
}
