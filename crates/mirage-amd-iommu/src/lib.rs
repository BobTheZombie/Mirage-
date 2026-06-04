#![no_std]
#![forbid(unsafe_code)]

//! AMD IOMMU mechanism descriptors with capability-mediated service handoff.

use mirage_cap::{CapabilityObject, CapabilityRights, CapabilitySet};
use mirage_ipc::EndpointId;
use mirage_ryzen::RyzenProfile;

/// Mirage-visible IOMMU unit identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdIommuId(u64);

impl AmdIommuId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Device-table range controlled by an AMD IOMMU instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IommuDeviceTable {
    pub base: u64,
    pub length: u64,
}

impl IommuDeviceTable {
    pub const fn new(base: u64, length: u64) -> Self {
        Self { base, length }
    }
}

/// Capability-protected AMD IOMMU resources delegated by the supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AmdIommuResources {
    pub pci_device: u64,
    pub mmio_base: u64,
    pub mmio_length: u64,
    pub irq_line: u16,
    pub device_table: IommuDeviceTable,
}

impl AmdIommuResources {
    pub const fn new(
        pci_device: u64,
        mmio_base: u64,
        mmio_length: u64,
        irq_line: u16,
        device_table: IommuDeviceTable,
    ) -> Self {
        Self {
            pci_device,
            mmio_base,
            mmio_length,
            irq_line,
            device_table,
        }
    }

    pub fn validate_caps(&self, caps: &CapabilitySet) -> Result<(), mirage_cap::CapabilityError> {
        caps.check(
            CapabilityObject::PciDevice(self.pci_device),
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
            CapabilityObject::DmaBuffer {
                base: self.device_table.base,
                length: self.device_table.length,
            },
            CapabilityRights::read_write_io(),
        )?;
        caps.check(
            CapabilityObject::IrqLine(self.irq_line),
            CapabilityRights::io(),
        )
    }
}

/// Supervisor handoff record for a restartable AMD IOMMU service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdIommuHandoff {
    pub iommu_id: AmdIommuId,
    pub profile: RyzenProfile,
    pub service_endpoint: EndpointId,
    pub resources: AmdIommuResources,
}

impl AmdIommuHandoff {
    pub const fn new(
        iommu_id: AmdIommuId,
        profile: RyzenProfile,
        service_endpoint: EndpointId,
        resources: AmdIommuResources,
    ) -> Self {
        Self {
            iommu_id,
            profile,
            service_endpoint,
            resources,
        }
    }
}
