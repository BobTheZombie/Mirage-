#![no_std]
#![forbid(unsafe_code)]

//! Platform discovery facts for supervisor policy.
//!
//! `mirage-platform` sits above low-level mechanism crates and below the
//! supervisor. It detects and reports platform candidates, but it never starts
//! driver services, binds drivers to devices, grants authority, or chooses
//! recovery policy. The Mirage supervisor remains the owner of those decisions.

extern crate alloc;

use alloc::vec::Vec;

pub mod timer;

pub use timer::{
    calibrate_timer, monotonic_now, timer_frequency, ApicTimer, HpetTimer, PitFallbackTimer,
    PlatformTimer, ReferenceTimer, SelectedPlatformTimer, TimerDiscovery, TimerError, TimerKind,
    TscCalibration, TscCounter, TscTimer,
};

use mirage_amd64::{AmdCpuId, AmdFeatureSet, AmdVendor};
use mirage_cap::{CapabilityObject, CapabilityRights, CapabilitySet};
use mirage_ipc::EndpointId;
use mirage_ryzen::{
    RyzenCpuId, RyzenDetectionInput, RyzenFeatureProfile, RyzenGeneration, RyzenPlatform,
    RyzenQuirk, RyzenSocKind, RyzenSupportStatus,
};

pub use mirage_ryzen::{
    detect_pstate_support, read_power_state_mock, read_temperature_mock, AmdPowerState,
    AmdPstateInfo, AmdTelemetry, AmdTelemetryError, AmdThermalSensor,
};

/// Stable upper bound for platform event arrays used by supervisor no-alloc paths.
pub const MAX_PLATFORM_DEVICE_EVENTS: usize = 32;

/// CPU identity and architectural feature facts discovered through `mirage-amd64`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuInfo {
    pub cpuid: AmdCpuId,
    pub vendor: AmdVendor,
    pub family: u16,
    pub model: u16,
    pub stepping: u8,
    pub features: AmdFeatureSet,
    pub ryzen: RyzenPlatform,
}

impl CpuInfo {
    pub fn detect() -> Self {
        Self::from_cpuid(AmdCpuId::read())
    }

    pub fn from_cpuid(cpuid: AmdCpuId) -> Self {
        let (family, model, stepping) = cpuid.family_model_stepping();
        let vendor = cpuid.vendor();
        let ryzen = RyzenPlatform::from_detection_input(RyzenDetectionInput::new(
            vendor.as_bytes(),
            RyzenCpuId::from_leaf1_eax(cpuid.leaf(0x0000_0001).eax),
            None,
        ));

        Self {
            cpuid,
            vendor,
            family: family.0,
            model: model.0,
            stepping: stepping.0,
            features: cpuid.features(),
            ryzen,
        }
    }
}

/// AMD chipset candidate counts discovered through `mirage-amd-chipset`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ChipsetInfo {
    pub pci_root_bus: Option<u8>,
    pub host_bridges: usize,
    pub pcie_root_complexes: usize,
    pub ahci_controllers: usize,
    pub xhci_controllers: usize,
    pub iommu_candidates: usize,
    pub amdgpu_candidates: usize,
}

/// Timer selection result. Timer choice is a mechanism report; scheduler policy remains elsewhere.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerInfo {
    pub selected: Option<TimerKind>,
    pub frequency_hz: Option<u64>,
    pub monotonic_now_ns: Option<u64>,
    pub error: Option<TimerError>,
}

impl TimerInfo {
    pub fn select(cpu: CpuInfo) -> Self {
        match calibrate_timer(TimerDiscovery::new(cpu.cpuid, cpu.ryzen)) {
            Ok(timer) => Self {
                selected: Some(timer.kind()),
                frequency_hz: Some(timer.timer_frequency()),
                monotonic_now_ns: Some(timer.monotonic_now()),
                error: None,
            },
            Err(error) => Self {
                selected: None,
                frequency_hz: None,
                monotonic_now_ns: None,
                error: Some(error),
            },
        }
    }
}

/// Interrupt-controller facts derived from CPU feature discovery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterruptInfo {
    pub apic_present: bool,
    pub x2apic_present: bool,
    pub preferred: InterruptControllerKind,
}

impl InterruptInfo {
    pub const fn from_features(features: AmdFeatureSet) -> Self {
        let preferred = if features.x2apic {
            InterruptControllerKind::X2ApicCandidate
        } else if features.apic {
            InterruptControllerKind::LocalApicCandidate
        } else {
            InterruptControllerKind::LegacyPicFallback
        };

        Self {
            apic_present: features.apic,
            x2apic_present: features.x2apic,
            preferred,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptControllerKind {
    X2ApicCandidate,
    LocalApicCandidate,
    LegacyPicFallback,
}

/// IOMMU facts discovered from Ryzen feature classification and PCI scaffolding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IommuInfo {
    pub ryzen_support: RyzenSupportStatus,
    pub pci_capabilities: usize,
    pub candidates: usize,
}

impl IommuInfo {
    pub fn from_cpu(cpu: CpuInfo) -> Self {
        Self {
            ryzen_support: cpu
                .ryzen
                .supports_feature(RyzenFeatureProfile::IommuIsolation),
            pci_capabilities: 0,
            candidates: 0,
        }
    }
}

/// Compact PCI identity copied into discovery events so driver crates do not
/// need to depend on Ryzen classification crates.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PciFunctionInfo {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision_id: u8,
    pub bar0_base: Option<u64>,
    pub bar0_length: Option<u64>,
    pub irq_line: Option<u16>,
}

impl PciFunctionInfo {
    pub const fn capability_object_id(self) -> u64 {
        ((self.bus as u64) << 16) | ((self.device as u64) << 8) | self.function as u64
    }
}

/// Driver-service role suggested by discovery. This is not a binding decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DeviceCandidateRole {
    AhciStorage,
    NvmeStorage,
    XhciUsb,
    AmdGpuDisplay,
    AmdIommu,
}

impl DeviceCandidateRole {
    pub const fn service_hint(self) -> &'static str {
        match self {
            Self::AhciStorage => "storaged.ahci",
            Self::NvmeStorage => "storaged.nvme",
            Self::XhciUsb => "usbd.xhci",
            Self::AmdGpuDisplay => "displayd.amdgpu",
            Self::AmdIommu => "platform.amd-iommu",
        }
    }

    pub const fn handoff_contract(self) -> &'static str {
        match self {
            Self::AhciStorage => "mirage.platform.storage.ahci-candidate.v1",
            Self::NvmeStorage => "mirage.platform.storage.nvme-candidate.v1",
            Self::XhciUsb => "mirage.platform.usb.xhci-candidate.v1",
            Self::AmdGpuDisplay => "mirage.platform.display.amdgpu-candidate.v1",
            Self::AmdIommu => "mirage.platform.iommu.amd-candidate.v1",
        }
    }
}

/// Discovery event emitted to the supervisor. Events report candidates only;
/// services claim devices later through supervisor-granted capabilities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceDiscoveryEvent {
    DriverCandidate {
        role: DeviceCandidateRole,
        pci: PciFunctionInfo,
        service_hint: &'static str,
        handoff_contract: &'static str,
    },
    IommuCapability {
        pci: PciFunctionInfo,
        mmio_base: u64,
        pci_segment: u16,
        flags: u16,
    },
}

impl DeviceDiscoveryEvent {
    pub const fn driver_candidate(role: DeviceCandidateRole, pci: PciFunctionInfo) -> Self {
        Self::DriverCandidate {
            role,
            pci,
            service_hint: role.service_hint(),
            handoff_contract: role.handoff_contract(),
        }
    }

    pub const fn pci(self) -> PciFunctionInfo {
        match self {
            Self::DriverCandidate { pci, .. } | Self::IommuCapability { pci, .. } => pci,
        }
    }
}

/// Complete discovery snapshot consumed by supervisor policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformInfo {
    pub cpu: CpuInfo,
    pub chipset: ChipsetInfo,
    pub timer: TimerInfo,
    pub interrupts: InterruptInfo,
    pub iommu: IommuInfo,
    pub ryzen_generation: RyzenGeneration,
    pub ryzen_soc: RyzenSocKind,
    pub events: Vec<DeviceDiscoveryEvent>,
}

/// Stateless platform discovery service. It reports facts and candidates only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformService {
    info: PlatformInfo,
}

impl PlatformService {
    pub fn detect() -> Self {
        Self::from_cpu(CpuInfo::detect())
    }

    pub fn from_cpu(cpu: CpuInfo) -> Self {
        let timer = TimerInfo::select(cpu);
        let interrupts = InterruptInfo::from_features(cpu.features);
        let iommu = IommuInfo::from_cpu(cpu);
        let info = PlatformInfo {
            cpu,
            chipset: ChipsetInfo::default(),
            timer,
            interrupts,
            iommu,
            ryzen_generation: cpu.ryzen.generation(),
            ryzen_soc: cpu.ryzen.soc_kind(),
            events: Vec::new(),
        };
        Self { info }
    }

    pub fn info(&self) -> &PlatformInfo {
        &self.info
    }

    pub fn into_info(self) -> PlatformInfo {
        self.info
    }

    pub fn device_discovery_events(&self) -> &[DeviceDiscoveryEvent] {
        &self.info.events
    }

    pub fn ryzen_requires_quirk(&self, quirk: RyzenQuirk) -> bool {
        self.info.cpu.ryzen.requires_quirk(quirk)
    }
}

#[cfg(feature = "hw-pci")]
impl PlatformService {
    /// Extend a CPU-only platform snapshot with PCI-backed AMD chipset and IOMMU candidates.
    pub fn discover_pci_bus(mut self, pci_bus: &mirage_pci::PciBus) -> Self {
        self.discover_amd_chipset(pci_bus);
        self.discover_iommu_scaffolding(pci_bus);
        self
    }

    fn discover_amd_chipset(&mut self, pci_bus: &mirage_pci::PciBus) {
        #[cfg(feature = "hw-amd-chipset")]
        {
            let chipset = mirage_amd_chipset::AmdChipset::discover(pci_bus);
            self.info.chipset = ChipsetInfo {
                pci_root_bus: Some(chipset.soc().pci_root().bus()),
                host_bridges: chipset.soc().host_bridges().len(),
                pcie_root_complexes: chipset.soc().pcie_root_complexes().len(),
                ahci_controllers: chipset.storage_controllers().len(),
                xhci_controllers: chipset.usb_controllers().len(),
                iommu_candidates: chipset.iommu_units().len(),
                amdgpu_candidates: chipset.gpu_candidates().len(),
            };
        }

        for device in pci_bus.devices() {
            let pci = pci_info(device);
            if device.is_ahci() {
                self.info
                    .events
                    .push(DeviceDiscoveryEvent::driver_candidate(
                        DeviceCandidateRole::AhciStorage,
                        pci,
                    ));
            } else if device.is_nvme() {
                self.info
                    .events
                    .push(DeviceDiscoveryEvent::driver_candidate(
                        DeviceCandidateRole::NvmeStorage,
                        pci,
                    ));
            } else if device.is_xhci() {
                self.info
                    .events
                    .push(DeviceDiscoveryEvent::driver_candidate(
                        DeviceCandidateRole::XhciUsb,
                        pci,
                    ));
            } else if device.is_amdgpu() {
                self.info
                    .events
                    .push(DeviceDiscoveryEvent::driver_candidate(
                        DeviceCandidateRole::AmdGpuDisplay,
                        pci,
                    ));
            }
        }
    }

    fn discover_iommu_scaffolding(&mut self, pci_bus: &mirage_pci::PciBus) {
        #[cfg(feature = "hw-amd-iommu")]
        {
            if let Ok(capabilities) = mirage_amd_iommu::discover_iommu_from_pci(pci_bus.devices()) {
                self.info.iommu.pci_capabilities = capabilities.len();
                for capability in capabilities {
                    if let Some(device) = pci_bus.devices().iter().find(|device| {
                        let address = device.address();
                        address.bus() == capability.device_id.bus()
                            && address.device() == capability.device_id.device()
                            && address.function() == capability.device_id.function()
                    }) {
                        let pci = pci_info(device);
                        self.info.iommu.candidates += 1;
                        self.info
                            .events
                            .push(DeviceDiscoveryEvent::driver_candidate(
                                DeviceCandidateRole::AmdIommu,
                                pci,
                            ));
                        self.info
                            .events
                            .push(DeviceDiscoveryEvent::IommuCapability {
                                pci,
                                mmio_base: capability.mmio_base,
                                pci_segment: capability.pci_segment,
                                flags: capability.flags,
                            });
                    }
                }
            }
        }
    }
}

#[cfg(feature = "hw-pci")]
fn pci_info(device: &mirage_pci::PciDevice) -> PciFunctionInfo {
    let address = device.address();
    let class = device.class_code();
    let bar0 = device.bar(0);
    let irq = device.header().interrupt_line();
    PciFunctionInfo {
        bus: address.bus(),
        device: address.device(),
        function: address.function(),
        vendor_id: device.vendor_id().get(),
        device_id: device.device_id().get(),
        class: class.class().get(),
        subclass: class.subclass().get(),
        prog_if: class.prog_if().get(),
        revision_id: device.revision_id(),
        bar0_base: bar0.map(|bar| bar.base()),
        bar0_length: bar0.and_then(|bar| bar.length()),
        irq_line: if irq == 0xff { None } else { Some(irq as u16) },
    }
}

/// Platform services that may be launched under supervisor control.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum PlatformServiceKind {
    AmdChipset,
    AmdIommu,
    AmdTelemetry,
}

/// Restart behavior requested from the Mirage supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartPolicy {
    RestartOnCrash,
    ManualRecovery,
}

/// Generic supervised-driver launch request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceLaunchRequest {
    pub kind: PlatformServiceKind,
    pub endpoint: EndpointId,
    pub restart_policy: RestartPolicy,
}

impl ServiceLaunchRequest {
    pub const fn new(
        kind: PlatformServiceKind,
        endpoint: EndpointId,
        restart_policy: RestartPolicy,
    ) -> Self {
        Self {
            kind,
            endpoint,
            restart_policy,
        }
    }
}

/// Capability bundle handed to a supervised platform driver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupervisorHandoff {
    pub launch: ServiceLaunchRequest,
    pub capabilities: CapabilitySet,
}

impl SupervisorHandoff {
    pub const fn new(launch: ServiceLaunchRequest, capabilities: CapabilitySet) -> Self {
        Self {
            launch,
            capabilities,
        }
    }

    pub fn validate_endpoint_capability(&self) -> Result<(), mirage_cap::CapabilityError> {
        self.capabilities.check(
            CapabilityObject::IpcEndpoint(self.launch.endpoint.get()),
            CapabilityRights::ipc(),
        )
    }
}

/// Policy planner for AMD platform services.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AmdPlatformPolicy;

impl AmdPlatformPolicy {
    pub const fn chipset_service(endpoint: EndpointId) -> ServiceLaunchRequest {
        ServiceLaunchRequest::new(
            PlatformServiceKind::AmdChipset,
            endpoint,
            RestartPolicy::RestartOnCrash,
        )
    }

    pub const fn iommu_service(endpoint: EndpointId) -> ServiceLaunchRequest {
        ServiceLaunchRequest::new(
            PlatformServiceKind::AmdIommu,
            endpoint,
            RestartPolicy::RestartOnCrash,
        )
    }

    pub const fn telemetry_service(endpoint: EndpointId) -> ServiceLaunchRequest {
        ServiceLaunchRequest::new(
            PlatformServiceKind::AmdTelemetry,
            endpoint,
            RestartPolicy::ManualRecovery,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirage_amd64::{AmdCpuidReader, CpuidLeaf};

    struct MockCpu;

    impl AmdCpuidReader for MockCpu {
        fn cpuid(&self, leaf: u32, _subleaf: u32) -> CpuidLeaf {
            match leaf {
                0x0000_0000 => CpuidLeaf::new(1, 0x6874_7541, 0x444d_4163, 0x6974_6e65),
                0x0000_0001 => CpuidLeaf::new(0x0087_0f10, 0, 1 << 21, (1 << 9) | (1 << 4)),
                0x8000_0000 => CpuidLeaf::new(0x8000_0008, 0, 0, 0),
                0x8000_0007 => CpuidLeaf::new(0, 0, 0, 1 << 8),
                _ => CpuidLeaf::default(),
            }
        }
    }

    #[test]
    fn platform_service_classifies_cpu_and_reports_timer_candidate() {
        let cpu = CpuInfo::from_cpuid(AmdCpuId::from_reader(&MockCpu));
        let service = PlatformService::from_cpu(cpu);

        assert_eq!(service.info().ryzen_generation, RyzenGeneration::Zen2);
        assert_eq!(
            service.info().interrupts.preferred,
            InterruptControllerKind::X2ApicCandidate
        );
        assert_eq!(service.info().timer.selected, Some(TimerKind::PitFallback));
    }
}
