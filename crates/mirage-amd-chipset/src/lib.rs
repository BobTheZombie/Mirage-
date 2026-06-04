#![no_std]
#![forbid(unsafe_code)]

//! AMD chipset descriptors used for supervisor-mediated driver handoff.

use mirage_cap::{CapabilityObject, CapabilityRights, CapabilitySet};
use mirage_ipc::EndpointId;
use mirage_ryzen::RyzenProfile;

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
