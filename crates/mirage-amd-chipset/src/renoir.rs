#![allow(dead_code)]

//! Renoir / Ryzen 5 4500U AMD SoC PCI inventory model.
//!
//! This module produces typed hardware candidates only.  It intentionally
//! does not reset devices, enable DMA, bind a driver, or mark anything Online.

use alloc::vec::Vec;
use mirage_pci::{PciClassCode, PciDevice, PciDeviceId, PciVendorId};

use crate::{
    AmdControllerRole, AmdDeviceCandidate, AmdPciFunction, AMD_CHIPSET_VENDOR_ID,
    AMD_DISPLAY_VENDOR_ID,
};

pub const RENOIR_SOC_NAME: &str = "AMD Renoir / Ryzen 4000U SoC";
pub const RYZEN_4500U_SOC_NAME: &str = "AMD Ryzen 5 4500U Renoir mobile APU";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RenoirPciRole {
    HostBridge,
    RootComplex,
    Iommu,
    Psp,
    AmdGpu,
    Xhci,
    Ahci,
    Nvme,
    HdAudio,
    AcpAudio,
    SmbusI2c,
    UnknownAmd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RenoirDeviceCandidate {
    pub function: AmdPciFunction,
    pub role: RenoirPciRole,
    pub service_hint: &'static str,
    pub handoff_contract: &'static str,
}

impl RenoirDeviceCandidate {
    pub const fn new(function: AmdPciFunction, role: RenoirPciRole) -> Self {
        Self {
            function,
            role,
            service_hint: service_hint(role),
            handoff_contract: handoff_contract(role),
        }
    }

    pub const fn as_amd_device_candidate(self) -> Option<AmdDeviceCandidate> {
        match self.role {
            RenoirPciRole::Iommu => Some(AmdDeviceCandidate::new(
                self.function,
                AmdControllerRole::Iommu,
            )),
            RenoirPciRole::AmdGpu => Some(AmdDeviceCandidate::new(
                self.function,
                AmdControllerRole::AmdGpuDisplay,
            )),
            RenoirPciRole::Xhci => Some(AmdDeviceCandidate::new(
                self.function,
                AmdControllerRole::XhciUsb,
            )),
            RenoirPciRole::Ahci => Some(AmdDeviceCandidate::new(
                self.function,
                AmdControllerRole::AhciStorage,
            )),
            RenoirPciRole::Nvme => Some(AmdDeviceCandidate::new(
                self.function,
                AmdControllerRole::NvmeStorage,
            )),
            RenoirPciRole::Psp => Some(AmdDeviceCandidate::new(
                self.function,
                AmdControllerRole::PspSecurityProcessor,
            )),
            RenoirPciRole::SmbusI2c => Some(AmdDeviceCandidate::new(
                self.function,
                AmdControllerRole::SmbusI2c,
            )),
            RenoirPciRole::HdAudio => Some(AmdDeviceCandidate::new(
                self.function,
                AmdControllerRole::AudioController,
            )),
            RenoirPciRole::AcpAudio => Some(AmdDeviceCandidate::new(
                self.function,
                AmdControllerRole::AcpAudioDmic,
            )),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenoirSocInventory {
    pub candidates: Vec<RenoirDeviceCandidate>,
    pub host_bridge_count: usize,
    pub storage_count: usize,
    pub usb_count: usize,
    pub display_count: usize,
    pub iommu_count: usize,
    pub psp_count: usize,
}

impl RenoirSocInventory {
    pub const fn empty() -> Self {
        Self {
            candidates: Vec::new(),
            host_bridge_count: 0,
            storage_count: 0,
            usb_count: 0,
            display_count: 0,
            iommu_count: 0,
            psp_count: 0,
        }
    }

    pub fn push(&mut self, candidate: RenoirDeviceCandidate) {
        match candidate.role {
            RenoirPciRole::HostBridge | RenoirPciRole::RootComplex => self.host_bridge_count += 1,
            RenoirPciRole::Ahci | RenoirPciRole::Nvme => self.storage_count += 1,
            RenoirPciRole::Xhci => self.usb_count += 1,
            RenoirPciRole::AmdGpu => self.display_count += 1,
            RenoirPciRole::Iommu => self.iommu_count += 1,
            RenoirPciRole::Psp => self.psp_count += 1,
            _ => {}
        }
        self.candidates.push(candidate);
    }
}

pub fn classify_renoir_device(device: &PciDevice) -> Option<RenoirDeviceCandidate> {
    let function = AmdPciFunction::from_pci_device(device);
    let role = classify_renoir_role(device.vendor_id(), device.device_id(), device.class_code())?;
    Some(RenoirDeviceCandidate::new(function, role))
}

pub fn classify_renoir_role(
    vendor: PciVendorId,
    _device_id: PciDeviceId,
    class_code: PciClassCode,
) -> Option<RenoirPciRole> {
    if vendor == AMD_DISPLAY_VENDOR_ID && class_code.is_display_controller() {
        return Some(RenoirPciRole::AmdGpu);
    }
    if vendor != AMD_CHIPSET_VENDOR_ID {
        return None;
    }
    if class_code.is_xhci() {
        return Some(RenoirPciRole::Xhci);
    }
    if class_code.is_ahci() {
        return Some(RenoirPciRole::Ahci);
    }
    if class_code.is_nvme() {
        return Some(RenoirPciRole::Nvme);
    }
    let class = class_code.class().get();
    let subclass = class_code.subclass().get();
    match (class, subclass) {
        (0x06, 0x00) => Some(RenoirPciRole::HostBridge),
        (0x06, 0x04) => Some(RenoirPciRole::RootComplex),
        (0x08, 0x06) => Some(RenoirPciRole::Iommu),
        (0x08, 0x80) => Some(RenoirPciRole::Psp),
        (0x04, 0x03) => Some(RenoirPciRole::HdAudio),
        (0x04, 0x80) => Some(RenoirPciRole::AcpAudio),
        (0x0c, 0x05) | (0x0c, 0x80) => Some(RenoirPciRole::SmbusI2c),
        _ => Some(RenoirPciRole::UnknownAmd),
    }
}

pub const fn service_hint(role: RenoirPciRole) -> &'static str {
    match role {
        RenoirPciRole::HostBridge | RenoirPciRole::RootComplex => "platform.renoir-root",
        RenoirPciRole::Iommu => "platform.amd-iommu",
        RenoirPciRole::Psp => "platform.amd-psp",
        RenoirPciRole::AmdGpu => "displayd.amdgpu.renoir",
        RenoirPciRole::Xhci => "usbd.xhci.renoir",
        RenoirPciRole::Ahci => "storaged.ahci",
        RenoirPciRole::Nvme => "storaged.nvme",
        RenoirPciRole::HdAudio => "audiod.hda",
        RenoirPciRole::AcpAudio => "audiod.amd-acp",
        RenoirPciRole::SmbusI2c => "platform.amd-smbus-i2c",
        RenoirPciRole::UnknownAmd => "platform.amd-unknown",
    }
}

pub const fn handoff_contract(role: RenoirPciRole) -> &'static str {
    match role {
        RenoirPciRole::HostBridge | RenoirPciRole::RootComplex => {
            "mirage.platform.renoir.root-complex.v1"
        }
        RenoirPciRole::Iommu => "mirage.platform.renoir.iommu.v1",
        RenoirPciRole::Psp => "mirage.platform.renoir.psp.v1",
        RenoirPciRole::AmdGpu => "mirage.platform.renoir.amdgpu.v1",
        RenoirPciRole::Xhci => "mirage.platform.renoir.xhci.v1",
        RenoirPciRole::Ahci => "mirage.platform.storage.ahci-candidate.v1",
        RenoirPciRole::Nvme => "mirage.platform.storage.nvme-candidate.v1",
        RenoirPciRole::HdAudio => "mirage.platform.audio.hda-candidate.v1",
        RenoirPciRole::AcpAudio => "mirage.platform.audio.amd-acp-dmic-candidate.v1",
        RenoirPciRole::SmbusI2c => "mirage.platform.bus.amd-smbus-i2c-candidate.v1",
        RenoirPciRole::UnknownAmd => "mirage.platform.amd.unknown-candidate.v1",
    }
}
