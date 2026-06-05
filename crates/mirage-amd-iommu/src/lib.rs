#![no_std]
#![forbid(unsafe_code)]

//! AMD IOMMU mechanism descriptors with capability-mediated service handoff.
//!
//! This crate intentionally models IOMMU discovery, domains, command/event
//! rings, and device-table encoding without globally enabling translation or
//! programming MMIO registers. Real register layouts are staged behind the
//! `hw-amd-iommu` feature and are only layout-annotated where hardware ABI
//! boundaries justify it.

extern crate alloc;

use alloc::vec::Vec;

use mirage_cap::{CapabilityObject, CapabilityRights, CapabilitySet};
use mirage_ipc::EndpointId;
use mirage_pci::{PciDevice, PciError};
use mirage_ryzen::RyzenProfile;

/// AMD IOMMU PCI capability identifier.
pub const AMD_IOMMU_PCI_CAPABILITY_ID: u8 = 0x0f;
const PCI_CAPABILITY_POINTER: u16 = 0x34;
const PCI_CAPABILITY_NEXT_MASK: u8 = 0xfc;
const MAX_PCI_CAPABILITY_HOPS: usize = 48;

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

/// PCI requester identifier for a device visible to the IOMMU.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DeviceId(u16);

impl DeviceId {
    pub const MAX_DEVICE: u8 = 31;
    pub const MAX_FUNCTION: u8 = 7;

    pub const fn new(bus: u8, device: u8, function: u8) -> Result<Self, AmdIommuError> {
        if device > Self::MAX_DEVICE {
            Err(AmdIommuError::InvalidDeviceId)
        } else if function > Self::MAX_FUNCTION {
            Err(AmdIommuError::InvalidDeviceId)
        } else {
            Ok(Self(
                ((bus as u16) << 8) | ((device as u16) << 3) | function as u16,
            ))
        }
    }

    pub const fn from_requester_id(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn requester_id(self) -> u16 {
        self.0
    }

    pub const fn bus(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub const fn device(self) -> u8 {
        ((self.0 >> 3) & 0x1f) as u8
    }

    pub const fn function(self) -> u8 {
        (self.0 & 0x7) as u8
    }
}

/// DMA bus address used in IOMMU domain mappings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DmaAddress(u64);

impl DmaAddress {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn checked_add(self, length: u64) -> Option<Self> {
        match self.0.checked_add(length) {
            Some(value) => Some(Self(value)),
            None => None,
        }
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

/// Parsed AMD IOMMU PCI capability record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AmdIommuCapability {
    pub capability_offset: u8,
    pub mmio_base: u64,
    pub pci_segment: u16,
    pub device_id: DeviceId,
    pub flags: u16,
}

impl AmdIommuCapability {
    pub const fn new(
        capability_offset: u8,
        mmio_base: u64,
        pci_segment: u16,
        device_id: DeviceId,
        flags: u16,
    ) -> Self {
        Self {
            capability_offset,
            mmio_base,
            pci_segment,
            device_id,
            flags,
        }
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

/// A domain identifier allocated by the supervisor policy layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdIommuDomainId(u16);

impl AmdIommuDomainId {
    pub const fn new(raw: u16) -> Result<Self, AmdIommuError> {
        if raw == 0 {
            Err(AmdIommuError::InvalidDomain)
        } else {
            Ok(Self(raw))
        }
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

/// A capability-checked DMA mapping inside an IOMMU domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AmdIommuMapping {
    pub dma_start: DmaAddress,
    pub physical_start: u64,
    pub length: u64,
    pub writable: bool,
    pub executable: bool,
}

impl AmdIommuMapping {
    pub const fn new(
        dma_start: DmaAddress,
        physical_start: u64,
        length: u64,
        writable: bool,
        executable: bool,
    ) -> Self {
        Self {
            dma_start,
            physical_start,
            length,
            writable,
            executable,
        }
    }

    pub const fn dma_object(self) -> CapabilityObject {
        CapabilityObject::DmaBuffer {
            base: self.physical_start,
            length: self.length,
        }
    }

    fn end(self) -> Result<DmaAddress, AmdIommuError> {
        self.dma_start
            .checked_add(self.length)
            .ok_or(AmdIommuError::InvalidMapping)
    }

    fn overlaps(self, other: Self) -> Result<bool, AmdIommuError> {
        let self_end = self.end()?.get();
        let other_end = other.end()?.get();
        Ok(self.dma_start.get() < other_end && other.dma_start.get() < self_end)
    }
}

/// Per-domain mapping and assignment state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdIommuDomain {
    id: AmdIommuDomainId,
    mappings: Vec<AmdIommuMapping>,
    devices: Vec<DeviceId>,
}

impl AmdIommuDomain {
    pub const fn new(id: AmdIommuDomainId) -> Self {
        Self {
            id,
            mappings: Vec::new(),
            devices: Vec::new(),
        }
    }

    pub const fn id(&self) -> AmdIommuDomainId {
        self.id
    }

    pub fn mappings(&self) -> &[AmdIommuMapping] {
        &self.mappings
    }

    pub fn devices(&self) -> &[DeviceId] {
        &self.devices
    }
}

/// AMD IOMMU device-table descriptor and mock-encoded entries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdIommuDeviceTable {
    entries: Vec<Option<AmdIommuDeviceTableEntry>>,
}

impl AmdIommuDeviceTable {
    pub fn new(entries: usize) -> Self {
        Self {
            entries: alloc::vec![None; entries],
        }
    }

    pub fn entry(&self, device: DeviceId) -> Option<AmdIommuDeviceTableEntry> {
        self.entries
            .get(device.requester_id() as usize)
            .and_then(|entry| *entry)
    }

    pub fn assign(
        &mut self,
        device: DeviceId,
        domain: AmdIommuDomainId,
        root_table: u64,
    ) -> Result<(), AmdIommuError> {
        if root_table & 0xfff != 0 {
            return Err(AmdIommuError::InvalidDeviceTableEntry);
        }
        let entry = self
            .entries
            .get_mut(device.requester_id() as usize)
            .ok_or(AmdIommuError::DeviceTableTooSmall)?;
        *entry = Some(AmdIommuDeviceTableEntry::new(domain, root_table));
        Ok(())
    }
}

/// Mock software view of a device-table entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AmdIommuDeviceTableEntry {
    domain: AmdIommuDomainId,
    root_table: u64,
    valid: bool,
}

impl AmdIommuDeviceTableEntry {
    pub const fn new(domain: AmdIommuDomainId, root_table: u64) -> Self {
        Self {
            domain,
            root_table,
            valid: true,
        }
    }

    pub const fn domain(self) -> AmdIommuDomainId {
        self.domain
    }

    pub const fn root_table(self) -> u64 {
        self.root_table
    }

    pub const fn is_valid(self) -> bool {
        self.valid
    }

    /// Encodes the fields Mirage currently owns into a stable mock format.
    /// This is not a hardware descriptor; real table layout is feature-gated.
    pub const fn encode_mock(self) -> u128 {
        let valid = if self.valid { 1u128 } else { 0u128 };
        valid | ((self.domain.get() as u128) << 16) | (((self.root_table >> 12) as u128) << 32)
    }
}

/// Command kind tracked by the mock command buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmdIommuCommand {
    InvalidateDevice { device: DeviceId },
    InvalidateDomain { domain: AmdIommuDomainId },
    CompleteWait,
}

/// Bounded software model of an AMD IOMMU command buffer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdIommuCommandBuffer {
    pending: Vec<CommandState>,
    default_complete_after_polls: u32,
}

impl AmdIommuCommandBuffer {
    pub const fn new(default_complete_after_polls: u32) -> Self {
        Self {
            pending: Vec::new(),
            default_complete_after_polls,
        }
    }

    pub fn submit(&mut self, command: AmdIommuCommand) -> CommandTicket {
        self.submit_with_completion(command, self.default_complete_after_polls)
    }

    pub fn submit_with_completion(
        &mut self,
        command: AmdIommuCommand,
        complete_after_polls: u32,
    ) -> CommandTicket {
        let ticket = CommandTicket(self.pending.len() as u64 + 1);
        self.pending.push(CommandState {
            ticket,
            command,
            polls_remaining: complete_after_polls,
            completed: false,
        });
        ticket
    }

    pub fn poll(&mut self, ticket: CommandTicket) -> Result<CommandStatus, AmdIommuError> {
        let state = self
            .pending
            .iter_mut()
            .find(|state| state.ticket == ticket)
            .ok_or(AmdIommuError::UnknownCommand)?;
        if state.completed {
            return Ok(CommandStatus::Completed);
        }
        if state.polls_remaining == 0 {
            state.completed = true;
            Ok(CommandStatus::Completed)
        } else {
            state.polls_remaining -= 1;
            Ok(CommandStatus::Pending)
        }
    }

    pub fn wait_for_completion(
        &mut self,
        ticket: CommandTicket,
        timeout_polls: u32,
    ) -> Result<(), AmdIommuError> {
        for _ in 0..timeout_polls {
            if self.poll(ticket)? == CommandStatus::Completed {
                return Ok(());
            }
        }
        Err(AmdIommuError::CommandTimeout { ticket })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CommandTicket(u64);

impl CommandTicket {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandStatus {
    Pending,
    Completed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CommandState {
    ticket: CommandTicket,
    command: AmdIommuCommand,
    polls_remaining: u32,
    completed: bool,
}

/// Event log used by tests and early service integration before IRQ plumbing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdIommuEventLog {
    events: Vec<AmdIommuEvent>,
    capacity: usize,
}

impl AmdIommuEventLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            events: Vec::new(),
            capacity,
        }
    }

    pub fn push(&mut self, event: AmdIommuEvent) -> Result<(), AmdIommuError> {
        if self.events.len() >= self.capacity {
            Err(AmdIommuError::EventLogFull)
        } else {
            self.events.push(event);
            Ok(())
        }
    }

    pub fn events(&self) -> &[AmdIommuEvent] {
        &self.events
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmdIommuEvent {
    DmaDenied {
        device: DeviceId,
        address: DmaAddress,
    },
    CommandCompleted {
        ticket: CommandTicket,
    },
}

/// Top-level software model of an AMD IOMMU unit. Translation is not enabled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdIommu {
    id: AmdIommuId,
    capability: AmdIommuCapability,
    device_table: AmdIommuDeviceTable,
    command_buffer: AmdIommuCommandBuffer,
    event_log: AmdIommuEventLog,
    domains: Vec<AmdIommuDomain>,
    translation_enabled: bool,
}

impl AmdIommu {
    pub fn new(
        id: AmdIommuId,
        capability: AmdIommuCapability,
        device_table_entries: usize,
    ) -> Self {
        Self {
            id,
            capability,
            device_table: AmdIommuDeviceTable::new(device_table_entries),
            command_buffer: AmdIommuCommandBuffer::new(1),
            event_log: AmdIommuEventLog::new(64),
            domains: Vec::new(),
            translation_enabled: false,
        }
    }

    pub const fn id(&self) -> AmdIommuId {
        self.id
    }

    pub const fn capability(&self) -> AmdIommuCapability {
        self.capability
    }

    pub const fn translation_enabled(&self) -> bool {
        self.translation_enabled
    }

    pub fn device_table(&self) -> &AmdIommuDeviceTable {
        &self.device_table
    }

    pub fn device_table_mut(&mut self) -> &mut AmdIommuDeviceTable {
        &mut self.device_table
    }

    pub fn command_buffer_mut(&mut self) -> &mut AmdIommuCommandBuffer {
        &mut self.command_buffer
    }

    pub fn event_log(&self) -> &AmdIommuEventLog {
        &self.event_log
    }

    pub fn domains(&self) -> &[AmdIommuDomain] {
        &self.domains
    }

    pub fn domain(&self, id: AmdIommuDomainId) -> Option<&AmdIommuDomain> {
        self.domains.iter().find(|domain| domain.id == id)
    }

    pub fn domain_mut(&mut self, id: AmdIommuDomainId) -> Option<&mut AmdIommuDomain> {
        self.domains.iter_mut().find(|domain| domain.id == id)
    }
}

/// Discover AMD IOMMU capabilities from a PCI enumeration snapshot.
pub fn discover_iommu_from_pci(
    devices: &[PciDevice],
) -> Result<Vec<AmdIommuCapability>, AmdIommuError> {
    let mut capabilities = Vec::new();
    for device in devices {
        if let Ok(capability) = parse_iommu_capability(device) {
            capabilities.push(capability);
        }
    }
    Ok(capabilities)
}

/// Parse the AMD IOMMU capability from one PCI function's capability list.
pub fn parse_iommu_capability(device: &PciDevice) -> Result<AmdIommuCapability, AmdIommuError> {
    let config = device.config_space();
    let mut offset = config.read_u8(PCI_CAPABILITY_POINTER)? & PCI_CAPABILITY_NEXT_MASK;
    let mut hops = 0usize;

    while offset != 0 && hops < MAX_PCI_CAPABILITY_HOPS {
        let cap_id = config.read_u8(offset as u16)?;
        let next = config.read_u8(offset as u16 + 1)? & PCI_CAPABILITY_NEXT_MASK;
        if cap_id == AMD_IOMMU_PCI_CAPABILITY_ID {
            let mmio_low = config.read_u32(offset as u16 + 4)? as u64;
            let mmio_high = config.read_u32(offset as u16 + 8)? as u64;
            let mmio_base = ((mmio_high << 32) | (mmio_low & 0xffff_c000)) & !0x3fff;
            let flags = config.read_u16(offset as u16 + 2)?;
            let pci_segment = config.read_u16(offset as u16 + 12)?;
            let address = device.address();
            return Ok(AmdIommuCapability::new(
                offset,
                mmio_base,
                pci_segment,
                DeviceId::new(address.bus(), address.device(), address.function())?,
                flags,
            ));
        }
        offset = next;
        hops += 1;
    }

    if hops >= MAX_PCI_CAPABILITY_HOPS {
        Err(AmdIommuError::MalformedPciCapability)
    } else {
        Err(AmdIommuError::IommuCapabilityNotFound)
    }
}

/// Create a new domain in an IOMMU unit.
pub fn create_domain(
    iommu: &mut AmdIommu,
    domain_id: AmdIommuDomainId,
) -> Result<(), AmdIommuError> {
    if iommu.domain(domain_id).is_some() {
        Err(AmdIommuError::DomainAlreadyExists)
    } else {
        iommu.domains.push(AmdIommuDomain::new(domain_id));
        Ok(())
    }
}

/// Map a DMA region after validating the service holds authority for it.
pub fn map_dma_region(
    domain: &mut AmdIommuDomain,
    mapping: AmdIommuMapping,
    caps: &CapabilitySet,
) -> Result<(), AmdIommuError> {
    if mapping.length == 0
        || mapping.physical_start & 0xfff != 0
        || mapping.dma_start.get() & 0xfff != 0
    {
        return Err(AmdIommuError::InvalidMapping);
    }

    caps.check(mapping.dma_object(), CapabilityRights::read_write_io())
        .map_err(|reason| AmdIommuError::DmaDenied {
            object: mapping.dma_object(),
            reason,
        })?;

    for existing in &domain.mappings {
        if existing.overlaps(mapping)? {
            return Err(AmdIommuError::MappingOverlap);
        }
    }

    domain.mappings.push(mapping);
    Ok(())
}

/// Remove an exact DMA mapping from a domain.
pub fn unmap_dma_region(
    domain: &mut AmdIommuDomain,
    dma_start: DmaAddress,
    length: u64,
) -> Result<AmdIommuMapping, AmdIommuError> {
    let index = domain
        .mappings
        .iter()
        .position(|mapping| mapping.dma_start == dma_start && mapping.length == length)
        .ok_or(AmdIommuError::MappingNotFound)?;
    Ok(domain.mappings.remove(index))
}

/// Assign a device to a domain and update the software device table.
pub fn assign_device_to_domain(
    iommu: &mut AmdIommu,
    device: DeviceId,
    domain_id: AmdIommuDomainId,
    root_table: u64,
) -> Result<(), AmdIommuError> {
    let domain = iommu
        .domain_mut(domain_id)
        .ok_or(AmdIommuError::DomainNotFound)?;
    if !domain.devices.contains(&device) {
        domain.devices.push(device);
    }
    iommu.device_table.assign(device, domain_id, root_table)
}

/// Structured errors returned by AMD IOMMU discovery and mock mechanism work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmdIommuError {
    Pci(PciError),
    IommuCapabilityNotFound,
    MalformedPciCapability,
    InvalidDeviceId,
    InvalidDomain,
    DomainAlreadyExists,
    DomainNotFound,
    InvalidMapping,
    MappingOverlap,
    MappingNotFound,
    DeviceTableTooSmall,
    InvalidDeviceTableEntry,
    UnknownCommand,
    CommandTimeout {
        ticket: CommandTicket,
    },
    EventLogFull,
    DmaDenied {
        object: CapabilityObject,
        reason: mirage_cap::CapabilityError,
    },
}

impl From<PciError> for AmdIommuError {
    fn from(value: PciError) -> Self {
        Self::Pci(value)
    }
}

/// Feature-gated real MMIO layouts. They are not used unless hardware bring-up
/// explicitly enables `hw-amd-iommu`.
#[cfg(feature = "hw-amd-iommu")]
pub mod hw {
    /// AMD IOMMU MMIO register block prefix. This is a hardware ABI boundary,
    /// so `repr(C)` is justified here; Mirage still does not globally enable
    /// translation from this crate.
    #[repr(C)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct AmdIommuMmioRegisters {
        pub device_table_base: u64,
        pub command_base: u64,
        pub event_base: u64,
        pub control: u64,
        pub exclusion_base: u64,
        pub exclusion_limit: u64,
    }

    /// Hardware command descriptor placeholder. The exact command opcode payload
    /// must be filled from public AMD documentation during hardware bring-up.
    #[repr(C)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct AmdIommuCommandDescriptor {
        pub words: [u32; 4],
    }

    /// Hardware event descriptor placeholder. Interrupt delivery and parsing are
    /// intentionally outside the default mock architecture skeleton.
    #[repr(C)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct AmdIommuEventDescriptor {
        pub words: [u32; 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use mirage_cap::Capability;
    use mirage_pci::{PciAddress, PciClassCode, PciConfigSpace, PciDeviceId, PciVendorId};

    fn iommu_device() -> PciDevice {
        let mut config = PciConfigSpace::endpoint(
            PciVendorId::AMD,
            PciDeviceId::new(0x1451),
            PciClassCode::from_raw(0x08, 0x06, 0x00),
            1,
        );
        config.write_u8(PCI_CAPABILITY_POINTER, 0x40).unwrap();
        config.write_u8(0x40, AMD_IOMMU_PCI_CAPABILITY_ID).unwrap();
        config.write_u8(0x41, 0).unwrap();
        config.write_u16(0x42, 0x1234).unwrap();
        config.write_u32(0x44, 0xfedc_4001).unwrap();
        config.write_u32(0x48, 0x0000_0001).unwrap();
        config.write_u16(0x4c, 2).unwrap();
        PciDevice::new(PciAddress::new(0, 2, 0).unwrap(), config).unwrap()
    }

    fn iommu() -> AmdIommu {
        AmdIommu::new(
            AmdIommuId::new(1),
            parse_iommu_capability(&iommu_device()).unwrap(),
            65536,
        )
    }

    fn dma_caps(base: u64, length: u64) -> CapabilitySet {
        CapabilitySet::from_capabilities(vec![Capability::new(
            CapabilityObject::DmaBuffer { base, length },
            CapabilityRights::read_write_io(),
        )])
    }

    #[test]
    fn parses_mock_iommu_capability() {
        let capability = parse_iommu_capability(&iommu_device()).unwrap();
        assert_eq!(capability.capability_offset, 0x40);
        assert_eq!(capability.mmio_base, 0x1_fedc_4000);
        assert_eq!(capability.pci_segment, 2);
        assert_eq!(capability.device_id, DeviceId::new(0, 2, 0).unwrap());
    }

    #[test]
    fn discovers_iommu_from_pci() {
        let devices = vec![iommu_device()];
        let discovered = discover_iommu_from_pci(&devices).unwrap();
        assert_eq!(discovered.len(), 1);
    }

    #[test]
    fn encodes_device_table_entry() {
        let domain = AmdIommuDomainId::new(7).unwrap();
        let entry = AmdIommuDeviceTableEntry::new(domain, 0x1234_5000);
        assert!(entry.is_valid());
        assert_eq!(entry.encode_mock(), 1 | (7u128 << 16) | (0x12345u128 << 32));
    }

    #[test]
    fn maps_and_unmaps_domain_regions() {
        let mut domain = AmdIommuDomain::new(AmdIommuDomainId::new(1).unwrap());
        let mapping = AmdIommuMapping::new(DmaAddress::new(0x2000), 0x8000, 0x1000, true, false);

        map_dma_region(&mut domain, mapping, &dma_caps(0x8000, 0x1000)).unwrap();
        assert_eq!(domain.mappings(), &[mapping]);

        let removed = unmap_dma_region(&mut domain, DmaAddress::new(0x2000), 0x1000).unwrap();
        assert_eq!(removed, mapping);
        assert!(domain.mappings().is_empty());
    }

    #[test]
    fn assigns_device_to_domain() {
        let mut iommu = iommu();
        let domain_id = AmdIommuDomainId::new(9).unwrap();
        create_domain(&mut iommu, domain_id).unwrap();
        let device = DeviceId::new(0, 4, 0).unwrap();

        assign_device_to_domain(&mut iommu, device, domain_id, 0x9000).unwrap();

        assert_eq!(iommu.domain(domain_id).unwrap().devices(), &[device]);
        assert_eq!(
            iommu.device_table().entry(device).unwrap().domain(),
            domain_id
        );
    }

    #[test]
    fn command_completion_times_out() {
        let mut commands = AmdIommuCommandBuffer::new(5);
        let ticket = commands.submit(AmdIommuCommand::CompleteWait);

        assert_eq!(
            commands.wait_for_completion(ticket, 2),
            Err(AmdIommuError::CommandTimeout { ticket })
        );
    }

    #[test]
    fn command_completion_can_be_polled_to_success() {
        let mut commands = AmdIommuCommandBuffer::new(1);
        let ticket = commands.submit(AmdIommuCommand::InvalidateDomain {
            domain: AmdIommuDomainId::new(3).unwrap(),
        });

        commands.wait_for_completion(ticket, 3).unwrap();
    }

    #[test]
    fn denies_dma_without_capability() {
        let mut domain = AmdIommuDomain::new(AmdIommuDomainId::new(1).unwrap());
        let mapping = AmdIommuMapping::new(DmaAddress::new(0x2000), 0x8000, 0x1000, true, false);
        let denied = map_dma_region(&mut domain, mapping, &CapabilitySet::new()).unwrap_err();

        assert_eq!(
            denied,
            AmdIommuError::DmaDenied {
                object: CapabilityObject::DmaBuffer {
                    base: 0x8000,
                    length: 0x1000,
                },
                reason: mirage_cap::CapabilityError::Missing,
            }
        );
        assert!(domain.mappings().is_empty());
    }
}
