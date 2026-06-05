#![no_std]
#![forbid(unsafe_code)]

//! AMD chipset discovery and supervisor handoff metadata.
//!
//! This crate deliberately stops at PCI identity and class-code discovery. It
//! does not bind AHCI, xHCI, IOMMU, or AMDGPU drivers directly; instead it emits
//! typed candidates that `mirage-platform` and supervisor policy can validate,
//! authorize, and route to restartable services.

extern crate alloc;

use alloc::vec::Vec;

use mirage_cap::{CapabilityObject, CapabilityRights, CapabilitySet};
use mirage_ipc::EndpointId;
use mirage_pci::{PciAddress, PciBus, PciClassCode, PciDevice, PciDeviceId, PciVendorId};
use mirage_ryzen::RyzenProfile;

/// AMD CPU/chipset PCI vendor identifier used by host bridges, FCH devices,
/// IOMMUs, SATA controllers, and USB controllers.
pub const AMD_CHIPSET_VENDOR_ID: PciVendorId = PciVendorId::new(0x1022);

/// AMD/ATI display PCI vendor identifier used by AMDGPU display functions.
pub const AMD_DISPLAY_VENDOR_ID: PciVendorId = PciVendorId::new(0x1002);

const CLASS_DISPLAY: u8 = 0x03;
const CLASS_BRIDGE: u8 = 0x06;
const CLASS_SYSTEM: u8 = 0x08;

const SUBCLASS_HOST_BRIDGE: u8 = 0x00;
const SUBCLASS_ISA_BRIDGE: u8 = 0x01;
const SUBCLASS_PCI_BRIDGE: u8 = 0x04;
const SUBCLASS_IOMMU: u8 = 0x06;
const SUBCLASS_VGA: u8 = 0x00;
const SUBCLASS_DISPLAY_OTHER: u8 = 0x80;

/// Mirage-visible chipset service identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdChipsetId(u64);

impl AmdChipsetId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Capability-protected chipset resources delegated by the supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AmdChipsetResources {
    pub pci_root: u64,
    pub mmio_base: u64,
    pub mmio_length: u64,
    pub irq_line: u16,
}

impl AmdChipsetResources {
    pub const fn new(pci_root: u64, mmio_base: u64, mmio_length: u64, irq_line: u16) -> Self {
        Self {
            pci_root,
            mmio_base,
            mmio_length,
            irq_line,
        }
    }

    pub fn validate_caps(&self, caps: &CapabilitySet) -> Result<(), mirage_cap::CapabilityError> {
        caps.check(
            CapabilityObject::PciDevice(self.pci_root),
            CapabilityRights::io(),
        )?;
        caps.check(
            CapabilityObject::MmioRegion {
                base: self.mmio_base,
                length: self.mmio_length,
            },
            CapabilityRights::read_write_io(),
        )?;
        caps.check(
            CapabilityObject::IrqLine(self.irq_line),
            CapabilityRights::io(),
        )
    }
}

/// Supervisor handoff record for a restartable AMD chipset driver service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdChipsetHandoff {
    pub chipset_id: AmdChipsetId,
    pub profile: RyzenProfile,
    pub service_endpoint: EndpointId,
    pub resources: AmdChipsetResources,
}

impl AmdChipsetHandoff {
    pub const fn new(
        chipset_id: AmdChipsetId,
        profile: RyzenProfile,
        service_endpoint: EndpointId,
        resources: AmdChipsetResources,
    ) -> Self {
        Self {
            chipset_id,
            profile,
            service_endpoint,
            resources,
        }
    }
}

/// Stable PCI function metadata suitable for supervisor handoff manifests.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdPciFunction {
    address: PciAddress,
    vendor_id: PciVendorId,
    device_id: PciDeviceId,
    class_code: PciClassCode,
    revision_id: u8,
}

impl AmdPciFunction {
    pub const fn new(
        address: PciAddress,
        vendor_id: PciVendorId,
        device_id: PciDeviceId,
        class_code: PciClassCode,
        revision_id: u8,
    ) -> Self {
        Self {
            address,
            vendor_id,
            device_id,
            class_code,
            revision_id,
        }
    }

    pub fn from_pci_device(device: &PciDevice) -> Self {
        Self::new(
            device.address(),
            device.vendor_id(),
            device.device_id(),
            device.class_code(),
            device.revision_id(),
        )
    }

    pub const fn address(self) -> PciAddress {
        self.address
    }

    pub const fn vendor_id(self) -> PciVendorId {
        self.vendor_id
    }

    pub const fn device_id(self) -> PciDeviceId {
        self.device_id
    }

    pub const fn class_code(self) -> PciClassCode {
        self.class_code
    }

    pub const fn revision_id(self) -> u8 {
        self.revision_id
    }
}

/// Platform-neutral role assigned to a discovered AMD PCI candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum AmdControllerRole {
    AhciStorage,
    XhciUsb,
    Iommu,
    AmdGpuDisplay,
}

impl AmdControllerRole {
    pub const fn service_hint(self) -> &'static str {
        match self {
            Self::AhciStorage => "storaged.ahci",
            Self::XhciUsb => "usbd.xhci",
            Self::Iommu => "platform.amd-iommu",
            Self::AmdGpuDisplay => "displayd.amdgpu",
        }
    }

    pub const fn handoff_contract(self) -> &'static str {
        match self {
            Self::AhciStorage => "mirage.platform.storage.ahci-candidate.v1",
            Self::XhciUsb => "mirage.platform.usb.xhci-candidate.v1",
            Self::Iommu => "mirage.platform.iommu.amd-candidate.v1",
            Self::AmdGpuDisplay => "mirage.platform.display.amdgpu-candidate.v1",
        }
    }
}

/// A driver-service candidate emitted for supervisor policy decisions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdDeviceCandidate {
    function: AmdPciFunction,
    role: AmdControllerRole,
    service_hint: &'static str,
    handoff_contract: &'static str,
}

impl AmdDeviceCandidate {
    pub const fn new(function: AmdPciFunction, role: AmdControllerRole) -> Self {
        Self {
            function,
            role,
            service_hint: role.service_hint(),
            handoff_contract: role.handoff_contract(),
        }
    }

    pub const fn function(self) -> AmdPciFunction {
        self.function
    }

    pub const fn role(self) -> AmdControllerRole {
        self.role
    }

    pub const fn service_hint(self) -> &'static str {
        self.service_hint
    }

    pub const fn handoff_contract(self) -> &'static str {
        self.handoff_contract
    }
}

/// Discovered AMD system-on-chip grouping for the scanned PCI root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdSoc {
    pci_root: AmdPciRoot,
    host_bridges: Vec<AmdHostBridge>,
    fch: Option<AmdFch>,
    pcie_root_complexes: Vec<AmdPcieRootComplex>,
}

impl AmdSoc {
    pub const fn new(pci_root: AmdPciRoot) -> Self {
        Self {
            pci_root,
            host_bridges: Vec::new(),
            fch: None,
            pcie_root_complexes: Vec::new(),
        }
    }

    pub const fn pci_root(&self) -> &AmdPciRoot {
        &self.pci_root
    }

    pub fn host_bridges(&self) -> &[AmdHostBridge] {
        &self.host_bridges
    }

    pub const fn fch(&self) -> Option<&AmdFch> {
        self.fch.as_ref()
    }

    pub fn pcie_root_complexes(&self) -> &[AmdPcieRootComplex] {
        &self.pcie_root_complexes
    }
}

/// AMD PCI root snapshot discovered from one enumerated `mirage-pci` bus.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdPciRoot {
    bus: u8,
}

impl AmdPciRoot {
    pub const fn new(bus: u8) -> Self {
        Self { bus }
    }

    pub const fn bus(self) -> u8 {
        self.bus
    }
}

/// AMD host bridge function.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdHostBridge {
    function: AmdPciFunction,
}

impl AmdHostBridge {
    pub const fn new(function: AmdPciFunction) -> Self {
        Self { function }
    }

    pub const fn function(self) -> AmdPciFunction {
        self.function
    }
}

/// AMD Fusion Controller Hub grouping surfaced to supervisor policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdFch {
    lpc_bridge: Option<AmdLpcBridge>,
    sata_controllers: Vec<AmdSataController>,
    usb_controllers: Vec<AmdUsbController>,
}

impl AmdFch {
    pub const fn empty() -> Self {
        Self {
            lpc_bridge: None,
            sata_controllers: Vec::new(),
            usb_controllers: Vec::new(),
        }
    }

    pub const fn lpc_bridge(&self) -> Option<AmdLpcBridge> {
        self.lpc_bridge
    }

    pub fn sata_controllers(&self) -> &[AmdSataController] {
        &self.sata_controllers
    }

    pub fn usb_controllers(&self) -> &[AmdUsbController] {
        &self.usb_controllers
    }
}

/// AMD PCIe root complex / downstream bridge function.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdPcieRootComplex {
    function: AmdPciFunction,
}

impl AmdPcieRootComplex {
    pub const fn new(function: AmdPciFunction) -> Self {
        Self { function }
    }

    pub const fn function(self) -> AmdPciFunction {
        self.function
    }
}

/// AMD AHCI SATA controller candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdSataController {
    candidate: AmdDeviceCandidate,
}

impl AmdSataController {
    pub const fn new(function: AmdPciFunction) -> Self {
        Self {
            candidate: AmdDeviceCandidate::new(function, AmdControllerRole::AhciStorage),
        }
    }

    pub const fn candidate(self) -> AmdDeviceCandidate {
        self.candidate
    }

    pub const fn is_ahci(self) -> bool {
        true
    }
}

/// AMD xHCI USB controller candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdUsbController {
    candidate: AmdDeviceCandidate,
}

impl AmdUsbController {
    pub const fn new(function: AmdPciFunction) -> Self {
        Self {
            candidate: AmdDeviceCandidate::new(function, AmdControllerRole::XhciUsb),
        }
    }

    pub const fn candidate(self) -> AmdDeviceCandidate {
        self.candidate
    }

    pub const fn is_xhci(self) -> bool {
        true
    }
}

/// AMD LPC/ISA bridge function that anchors FCH-style legacy platform glue.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdLpcBridge {
    function: AmdPciFunction,
}

impl AmdLpcBridge {
    pub const fn new(function: AmdPciFunction) -> Self {
        Self { function }
    }

    pub const fn function(self) -> AmdPciFunction {
        self.function
    }
}

/// AMD chipset discovery result for one PCI bus snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdChipset {
    soc: AmdSoc,
    storage_controllers: Vec<AmdSataController>,
    usb_controllers: Vec<AmdUsbController>,
    iommu_units: Vec<AmdDeviceCandidate>,
    gpu_candidates: Vec<AmdDeviceCandidate>,
}

impl AmdChipset {
    /// Discover AMD chipset and device-service candidates from an enumerated PCI bus.
    pub fn discover(pci_bus: &PciBus) -> Self {
        let mut soc = AmdSoc::new(AmdPciRoot::new(pci_bus.bus()));
        let mut fch = AmdFch::empty();
        let mut storage_controllers = Vec::new();
        let mut usb_controllers = Vec::new();
        let mut iommu_units = Vec::new();
        let mut gpu_candidates = Vec::new();

        for device in pci_bus.devices() {
            let function = AmdPciFunction::from_pci_device(device);

            if is_amd_chipset_device(device) && is_host_bridge(device.class_code()) {
                soc.host_bridges.push(AmdHostBridge::new(function));
            }

            if is_amd_chipset_device(device) && is_pci_bridge(device.class_code()) {
                soc.pcie_root_complexes
                    .push(AmdPcieRootComplex::new(function));
            }

            if is_amd_chipset_device(device) && is_lpc_bridge(device.class_code()) {
                fch.lpc_bridge = Some(AmdLpcBridge::new(function));
            }

            if is_amd_chipset_device(device) && device.is_ahci() {
                let controller = AmdSataController::new(function);
                fch.sata_controllers.push(controller);
                storage_controllers.push(controller);
            }

            if is_amd_chipset_device(device) && device.is_xhci() {
                let controller = AmdUsbController::new(function);
                fch.usb_controllers.push(controller);
                usb_controllers.push(controller);
            }

            if is_amd_chipset_device(device) && is_iommu(device.class_code()) {
                iommu_units.push(AmdDeviceCandidate::new(function, AmdControllerRole::Iommu));
            }

            if is_amd_gpu_device(device) {
                gpu_candidates.push(AmdDeviceCandidate::new(
                    function,
                    AmdControllerRole::AmdGpuDisplay,
                ));
            }
        }

        if fch.lpc_bridge.is_some()
            || !fch.sata_controllers.is_empty()
            || !fch.usb_controllers.is_empty()
        {
            soc.fch = Some(fch);
        }

        Self {
            soc,
            storage_controllers,
            usb_controllers,
            iommu_units,
            gpu_candidates,
        }
    }

    pub const fn soc(&self) -> &AmdSoc {
        &self.soc
    }

    pub fn storage_controllers(&self) -> &[AmdSataController] {
        &self.storage_controllers
    }

    pub fn usb_controllers(&self) -> &[AmdUsbController] {
        &self.usb_controllers
    }

    pub fn iommu_units(&self) -> &[AmdDeviceCandidate] {
        &self.iommu_units
    }

    pub fn gpu_candidates(&self) -> &[AmdDeviceCandidate] {
        &self.gpu_candidates
    }
}

fn is_amd_chipset_device(device: &PciDevice) -> bool {
    device.vendor_id().get() == AMD_CHIPSET_VENDOR_ID.get()
}

fn is_amd_gpu_device(device: &PciDevice) -> bool {
    device.vendor_id().get() == AMD_DISPLAY_VENDOR_ID.get()
        && is_display_gpu_candidate(device.class_code())
}

const fn is_host_bridge(class_code: PciClassCode) -> bool {
    class_code.class().get() == CLASS_BRIDGE && class_code.subclass().get() == SUBCLASS_HOST_BRIDGE
}

const fn is_pci_bridge(class_code: PciClassCode) -> bool {
    class_code.class().get() == CLASS_BRIDGE && class_code.subclass().get() == SUBCLASS_PCI_BRIDGE
}

const fn is_lpc_bridge(class_code: PciClassCode) -> bool {
    class_code.class().get() == CLASS_BRIDGE && class_code.subclass().get() == SUBCLASS_ISA_BRIDGE
}

const fn is_iommu(class_code: PciClassCode) -> bool {
    class_code.class().get() == CLASS_SYSTEM && class_code.subclass().get() == SUBCLASS_IOMMU
}

const fn is_display_gpu_candidate(class_code: PciClassCode) -> bool {
    class_code.class().get() == CLASS_DISPLAY
        && (class_code.subclass().get() == SUBCLASS_VGA
            || class_code.subclass().get() == SUBCLASS_DISPLAY_OTHER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirage_pci::{MockPciConfigAccess, PciConfigSpace};

    const CLASS_MASS_STORAGE: u8 = 0x01;
    const CLASS_SERIAL_BUS: u8 = 0x0c;
    const SUBCLASS_SATA: u8 = 0x06;
    const SUBCLASS_USB: u8 = 0x03;
    const PROGIF_AHCI: u8 = 0x01;
    const PROGIF_XHCI: u8 = 0x30;

    fn addr(device: u8, function: u8) -> PciAddress {
        PciAddress::new(0, device, function).unwrap()
    }

    fn endpoint(vendor: u16, device: u16, class: u8, subclass: u8, prog_if: u8) -> PciConfigSpace {
        PciConfigSpace::endpoint(
            PciVendorId::new(vendor),
            PciDeviceId::new(device),
            PciClassCode::from_raw(class, subclass, prog_if),
            0x01,
        )
    }

    fn amd_bus(functions: &[(u8, u16, u8, u8, u8)]) -> PciBus {
        let mut access = MockPciConfigAccess::new();
        for (slot, device_id, class, subclass, prog_if) in functions {
            access.add_function(
                addr(*slot, 0),
                endpoint(0x1022, *device_id, *class, *subclass, *prog_if),
            );
        }
        PciBus::enumerate(0, &access).unwrap()
    }

    #[test]
    fn discovers_amd_chipset_topology_from_mock_pci_tree() {
        let bus = amd_bus(&[
            (0, 0x14d8, CLASS_BRIDGE, SUBCLASS_HOST_BRIDGE, 0x00),
            (1, 0x14db, CLASS_BRIDGE, SUBCLASS_PCI_BRIDGE, 0x00),
            (20, 0x790e, CLASS_BRIDGE, SUBCLASS_ISA_BRIDGE, 0x00),
        ]);

        let chipset = AmdChipset::discover(&bus);

        assert_eq!(chipset.soc().pci_root().bus(), 0);
        assert_eq!(chipset.soc().host_bridges().len(), 1);
        assert_eq!(chipset.soc().pcie_root_complexes().len(), 1);
        assert!(chipset.soc().fch().unwrap().lpc_bridge().is_some());
    }

    #[test]
    fn detects_amd_ahci_storage_controller_candidate() {
        let bus = amd_bus(&[(17, 0x7901, CLASS_MASS_STORAGE, SUBCLASS_SATA, PROGIF_AHCI)]);

        let chipset = AmdChipset::discover(&bus);

        let storage = chipset.storage_controllers();
        assert_eq!(storage.len(), 1);
        assert!(storage[0].is_ahci());
        assert_eq!(
            storage[0].candidate().role(),
            AmdControllerRole::AhciStorage
        );
        assert_eq!(storage[0].candidate().service_hint(), "storaged.ahci");
    }

    #[test]
    fn detects_amd_xhci_usb_controller_candidate() {
        let bus = amd_bus(&[(16, 0x43f7, CLASS_SERIAL_BUS, SUBCLASS_USB, PROGIF_XHCI)]);

        let chipset = AmdChipset::discover(&bus);

        let usb = chipset.usb_controllers();
        assert_eq!(usb.len(), 1);
        assert!(usb[0].is_xhci());
        assert_eq!(usb[0].candidate().role(), AmdControllerRole::XhciUsb);
        assert_eq!(
            usb[0].candidate().handoff_contract(),
            "mirage.platform.usb.xhci-candidate.v1"
        );
    }

    #[test]
    fn detects_amd_iommu_candidate() {
        let bus = amd_bus(&[(0, 0x1451, CLASS_SYSTEM, SUBCLASS_IOMMU, 0x00)]);

        let chipset = AmdChipset::discover(&bus);

        let iommus = chipset.iommu_units();
        assert_eq!(iommus.len(), 1);
        assert_eq!(iommus[0].role(), AmdControllerRole::Iommu);
        assert_eq!(iommus[0].service_hint(), "platform.amd-iommu");
    }

    #[test]
    fn exposes_controller_candidates_without_driver_crate_dependencies() {
        let access = MockPciConfigAccess::new()
            .with_function(
                addr(1, 0),
                endpoint(
                    0x1022,
                    0x7901,
                    CLASS_MASS_STORAGE,
                    SUBCLASS_SATA,
                    PROGIF_AHCI,
                ),
            )
            .with_function(
                addr(2, 0),
                endpoint(0x1022, 0x43f7, CLASS_SERIAL_BUS, SUBCLASS_USB, PROGIF_XHCI),
            )
            .with_function(
                addr(3, 0),
                endpoint(0x1022, 0x1451, CLASS_SYSTEM, SUBCLASS_IOMMU, 0x00),
            )
            .with_function(
                addr(4, 0),
                endpoint(0x1002, 0x73bf, CLASS_DISPLAY, SUBCLASS_VGA, 0x00),
            )
            .with_function(
                addr(5, 0),
                endpoint(
                    0x8086,
                    0x2922,
                    CLASS_MASS_STORAGE,
                    SUBCLASS_SATA,
                    PROGIF_AHCI,
                ),
            );

        let bus = PciBus::enumerate(0, &access).unwrap();
        let chipset = AmdChipset::discover(&bus);

        assert_eq!(chipset.storage_controllers().len(), 1);
        assert_eq!(chipset.usb_controllers().len(), 1);
        assert_eq!(chipset.iommu_units().len(), 1);
        assert_eq!(chipset.gpu_candidates().len(), 1);
        assert_eq!(
            chipset.gpu_candidates()[0].handoff_contract(),
            "mirage.platform.display.amdgpu-candidate.v1"
        );
        assert_eq!(
            chipset.storage_controllers()[0]
                .candidate()
                .function()
                .address(),
            addr(1, 0)
        );
    }
}
