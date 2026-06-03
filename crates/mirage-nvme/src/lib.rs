#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use mirage_block::{
    BlockDevice, BlockDeviceId, BlockDeviceInfo, BlockDeviceState, BlockError, BlockRange,
    BlockSize, Lba, SectorCount,
};
use mirage_cap::{
    CapabilityError, CapabilityObject, CapabilityRight, CapabilityRights, CapabilitySet,
};
use mirage_ipc::EndpointId;

/// Mirage-visible identifier for an NVMe controller service instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct NvmeControllerId(u64);

impl NvmeControllerId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Mirage-visible NVMe namespace identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct NvmeNamespaceId(u32);

impl NvmeNamespaceId {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Capability-protected hardware resources required by this NVMe controller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NvmeHardwareResources {
    pub pci_device: u64,
    pub mmio_base: u64,
    pub mmio_length: u64,
    pub dma_region: u64,
    pub irq_line: u16,
}

impl NvmeHardwareResources {
    pub const fn new(
        pci_device: u64,
        mmio_base: u64,
        mmio_length: u64,
        dma_region: u64,
        irq_line: u16,
    ) -> Self {
        Self {
            pci_device,
            mmio_base,
            mmio_length,
            dma_region,
            irq_line,
        }
    }
}

/// Errors surfaced by the mock NVMe service layer before translation to block errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NvmeError {
    Capability(CapabilityError),
    InvalidBlockSize,
    NamespaceNotFound,
    BufferSizeMismatch,
    OutOfBounds,
    ReadOnly,
    Offline,
    Faulted,
}

impl From<CapabilityError> for NvmeError {
    fn from(error: CapabilityError) -> Self {
        Self::Capability(error)
    }
}

impl From<BlockError> for NvmeError {
    fn from(error: BlockError) -> Self {
        match error {
            BlockError::InvalidBlockSize => Self::InvalidBlockSize,
            BlockError::BufferSizeMismatch => Self::BufferSizeMismatch,
            BlockError::OutOfBounds | BlockError::EmptyRange | BlockError::RangeOverflow => {
                Self::OutOfBounds
            }
            BlockError::ReadOnly => Self::ReadOnly,
            BlockError::DeviceOffline => Self::Offline,
            BlockError::DeviceFaulted => Self::Faulted,
            BlockError::QueueEmpty | BlockError::DeviceMismatch | BlockError::Io => Self::Faulted,
        }
    }
}

impl From<NvmeError> for BlockError {
    fn from(error: NvmeError) -> Self {
        match error {
            NvmeError::InvalidBlockSize => Self::InvalidBlockSize,
            NvmeError::BufferSizeMismatch => Self::BufferSizeMismatch,
            NvmeError::OutOfBounds => Self::OutOfBounds,
            NvmeError::ReadOnly => Self::ReadOnly,
            NvmeError::Offline => Self::DeviceOffline,
            NvmeError::Faulted => Self::DeviceFaulted,
            NvmeError::NamespaceNotFound | NvmeError::Capability(_) => Self::Io,
        }
    }
}

/// Subset of NVMe identify data needed by Mirage storage discovery tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NvmeIdentifyData {
    pub model_number: [u8; 40],
    pub serial_number: [u8; 20],
    pub firmware_revision: [u8; 8],
    pub namespace_count: u32,
    pub supports_write_cache: bool,
}

impl NvmeIdentifyData {
    pub const fn new(
        model_number: [u8; 40],
        serial_number: [u8; 20],
        firmware_revision: [u8; 8],
        namespace_count: u32,
        supports_write_cache: bool,
    ) -> Self {
        Self {
            model_number,
            serial_number,
            firmware_revision,
            namespace_count,
            supports_write_cache,
        }
    }

    pub const fn mock(namespace_count: u32) -> Self {
        let mut model_number = [b' '; 40];
        model_number[0] = b'M';
        model_number[1] = b'i';
        model_number[2] = b'r';
        model_number[3] = b'a';
        model_number[4] = b'g';
        model_number[5] = b'e';
        model_number[6] = b' ';
        model_number[7] = b'N';
        model_number[8] = b'V';
        model_number[9] = b'M';
        model_number[10] = b'e';

        let mut serial_number = [b' '; 20];
        serial_number[0] = b'M';
        serial_number[1] = b'O';
        serial_number[2] = b'C';
        serial_number[3] = b'K';
        serial_number[4] = b'0';
        serial_number[5] = b'0';
        serial_number[6] = b'1';

        let mut firmware_revision = [b' '; 8];
        firmware_revision[0] = b'0';
        firmware_revision[1] = b'.';
        firmware_revision[2] = b'1';

        Self::new(
            model_number,
            serial_number,
            firmware_revision,
            namespace_count,
            true,
        )
    }
}

/// Minimal NVMe command descriptor used by the mock queue pair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NvmeCommand {
    pub command_id: u16,
    pub opcode: NvmeOpcode,
    pub namespace_id: Option<NvmeNamespaceId>,
    pub lba: Lba,
    pub blocks: SectorCount,
}

impl NvmeCommand {
    pub const fn identify(command_id: u16) -> Self {
        Self {
            command_id,
            opcode: NvmeOpcode::Identify,
            namespace_id: None,
            lba: Lba::new(0),
            blocks: SectorCount::new(0),
        }
    }

    pub const fn read(
        command_id: u16,
        namespace_id: NvmeNamespaceId,
        lba: Lba,
        blocks: SectorCount,
    ) -> Self {
        Self {
            command_id,
            opcode: NvmeOpcode::Read,
            namespace_id: Some(namespace_id),
            lba,
            blocks,
        }
    }

    pub const fn write(
        command_id: u16,
        namespace_id: NvmeNamespaceId,
        lba: Lba,
        blocks: SectorCount,
    ) -> Self {
        Self {
            command_id,
            opcode: NvmeOpcode::Write,
            namespace_id: Some(namespace_id),
            lba,
            blocks,
        }
    }
}

/// NVMe command opcodes represented by this mock driver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NvmeOpcode {
    Identify,
    Read,
    Write,
    Flush,
}

/// Minimal completion queue entry for a mock NVMe command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NvmeCompletion {
    pub command_id: u16,
    pub status: NvmeCompletionStatus,
}

impl NvmeCompletion {
    pub const fn success(command_id: u16) -> Self {
        Self {
            command_id,
            status: NvmeCompletionStatus::Success,
        }
    }

    pub const fn failed(command_id: u16, error: NvmeError) -> Self {
        Self {
            command_id,
            status: NvmeCompletionStatus::Failed(error),
        }
    }
}

/// Completion status for mock command execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NvmeCompletionStatus {
    Success,
    Failed(NvmeError),
}

/// Submission/completion queue pair owned by a supervised NVMe driver service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NvmeQueuePair {
    id: u16,
    endpoint: EndpointId,
    submitted: Vec<NvmeCommand>,
    completed: Vec<NvmeCompletion>,
}

impl NvmeQueuePair {
    pub const fn new(id: u16, endpoint: EndpointId) -> Self {
        Self {
            id,
            endpoint,
            submitted: Vec::new(),
            completed: Vec::new(),
        }
    }

    pub const fn id(&self) -> u16 {
        self.id
    }

    pub const fn endpoint(&self) -> EndpointId {
        self.endpoint
    }

    pub fn submit(&mut self, command: NvmeCommand) {
        self.submitted.push(command);
    }

    pub fn complete(&mut self, completion: NvmeCompletion) {
        self.completed.push(completion);
    }

    pub fn completions(&self) -> &[NvmeCompletion] {
        &self.completed
    }
}

/// Mock NVMe namespace discovered from controller identify data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NvmeNamespace {
    id: NvmeNamespaceId,
    block_device_id: BlockDeviceId,
    block_size: BlockSize,
    sectors: SectorCount,
    read_only: bool,
    storage: Vec<u8>,
}

impl NvmeNamespace {
    pub fn new(
        id: NvmeNamespaceId,
        block_device_id: BlockDeviceId,
        block_size: BlockSize,
        sectors: SectorCount,
        read_only: bool,
    ) -> Result<Self, NvmeError> {
        let bytes = BlockRange::new(Lba::new(0), sectors).byte_len(block_size)?;
        // TODO: namespace formatting must be implemented by the storage service policy layer.
        Ok(Self {
            id,
            block_device_id,
            block_size,
            sectors,
            read_only,
            storage: vec![0; bytes],
        })
    }

    pub const fn id(&self) -> NvmeNamespaceId {
        self.id
    }

    pub const fn block_device_id(&self) -> BlockDeviceId {
        self.block_device_id
    }

    pub const fn info(&self) -> BlockDeviceInfo {
        BlockDeviceInfo::new(
            self.block_device_id,
            self.block_size,
            self.sectors,
            self.read_only,
            true,
        )
    }

    fn range_bounds(&self, range: BlockRange) -> Result<(usize, usize), NvmeError> {
        let len = self.info().expected_buffer_len(range)?;
        let start_lba = usize::try_from(range.start().get()).map_err(|_| NvmeError::OutOfBounds)?;
        let start = start_lba
            .checked_mul(self.block_size.bytes_usize())
            .ok_or(NvmeError::OutOfBounds)?;
        let end = start.checked_add(len).ok_or(NvmeError::OutOfBounds)?;
        Ok((start, end))
    }
}

/// Mock NVMe controller service state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NvmeController {
    id: NvmeControllerId,
    resources: NvmeHardwareResources,
    authority: CapabilitySet,
    identify: NvmeIdentifyData,
    admin_queue: NvmeQueuePair,
    io_queue: NvmeQueuePair,
    namespaces: Vec<NvmeNamespace>,
    state: BlockDeviceState,
}

impl NvmeController {
    pub fn new(
        id: NvmeControllerId,
        resources: NvmeHardwareResources,
        authority: CapabilitySet,
        namespaces: Vec<NvmeNamespace>,
    ) -> Self {
        // TODO: PCI BAR mapping must be mediated by supervisor-granted PCI/MMIO capabilities.
        // TODO: admin queue setup must allocate controller-owned command buffers.
        // TODO: I/O queue setup must provision per-namespace submission/completion queues.
        // TODO: MSI/MSI-X interrupts must be wired through IRQ capabilities, not raw vectors.
        // TODO: DMA-safe memory must come from supervisor-approved DMA regions.
        // TODO: power management must be policy-driven by the supervisor.
        let identify = NvmeIdentifyData::mock(namespaces.len() as u32);
        Self {
            id,
            resources,
            authority,
            identify,
            admin_queue: NvmeQueuePair::new(0, EndpointId::new(id.get() << 32)),
            io_queue: NvmeQueuePair::new(1, EndpointId::new((id.get() << 32) | 1)),
            namespaces,
            state: BlockDeviceState::Online,
        }
    }

    pub const fn id(&self) -> NvmeControllerId {
        self.id
    }

    pub const fn resources(&self) -> NvmeHardwareResources {
        self.resources
    }

    pub const fn state(&self) -> BlockDeviceState {
        self.state
    }

    pub fn set_state(&mut self, state: BlockDeviceState) {
        self.state = state;
    }

    pub fn admin_queue(&self) -> &NvmeQueuePair {
        &self.admin_queue
    }

    pub fn io_queue(&self) -> &NvmeQueuePair {
        &self.io_queue
    }

    pub fn identify_controller(&mut self) -> Result<NvmeIdentifyData, NvmeError> {
        self.check_hardware_authority()?;
        let command = NvmeCommand::identify(1);
        self.admin_queue.submit(command.clone());
        // TODO: PRP/SGL mapping must translate identify buffers through DMA-safe memory.
        self.admin_queue
            .complete(NvmeCompletion::success(command.command_id));
        Ok(self.identify.clone())
    }

    pub fn discover_namespaces(&mut self) -> Result<&[NvmeNamespace], NvmeError> {
        self.check_hardware_authority()?;
        let command = NvmeCommand::identify(2);
        self.admin_queue.submit(command.clone());
        self.admin_queue
            .complete(NvmeCompletion::success(command.command_id));
        Ok(&self.namespaces)
    }

    pub fn into_block_device(
        self,
        namespace_id: NvmeNamespaceId,
    ) -> Result<NvmeBlockDevice, NvmeError> {
        let namespace = self
            .namespaces
            .into_iter()
            .find(|namespace| namespace.id() == namespace_id)
            .ok_or(NvmeError::NamespaceNotFound)?;
        Ok(NvmeBlockDevice::new(
            self.id,
            self.resources,
            self.authority,
            namespace,
            self.state,
        ))
    }

    fn check_hardware_authority(&self) -> Result<(), NvmeError> {
        check_hardware_authority(&self.authority, self.resources)
    }
}

/// BlockDevice adapter for a single NVMe namespace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NvmeBlockDevice {
    controller_id: NvmeControllerId,
    resources: NvmeHardwareResources,
    authority: CapabilitySet,
    namespace: NvmeNamespace,
    state: BlockDeviceState,
}

impl NvmeBlockDevice {
    pub const fn new(
        controller_id: NvmeControllerId,
        resources: NvmeHardwareResources,
        authority: CapabilitySet,
        namespace: NvmeNamespace,
        state: BlockDeviceState,
    ) -> Self {
        Self {
            controller_id,
            resources,
            authority,
            namespace,
            state,
        }
    }

    pub const fn controller_id(&self) -> NvmeControllerId {
        self.controller_id
    }

    pub const fn namespace_id(&self) -> NvmeNamespaceId {
        self.namespace.id()
    }

    pub fn identify_namespace(&self) -> Result<BlockDeviceInfo, NvmeError> {
        self.check_hardware_authority()?;
        Ok(self.namespace.info())
    }

    fn check_hardware_authority(&self) -> Result<(), NvmeError> {
        check_hardware_authority(&self.authority, self.resources)
    }
}

impl BlockDevice for NvmeBlockDevice {
    fn info(&self) -> BlockDeviceInfo {
        self.namespace.info()
    }

    fn state(&self) -> BlockDeviceState {
        self.state
    }

    fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.check_hardware_authority()?;
        self.validate_read(range, buffer)?;
        let (start, end) = self.namespace.range_bounds(range)?;
        buffer.copy_from_slice(&self.namespace.storage[start..end]);
        Ok(())
    }

    fn write_blocks(&mut self, range: BlockRange, data: &[u8]) -> Result<(), BlockError> {
        self.check_hardware_authority()?;
        self.validate_write(range, data)?;
        let (start, end) = self.namespace.range_bounds(range)?;
        self.namespace.storage[start..end].copy_from_slice(data);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.check_hardware_authority()?;
        self.state.ensure_available()
    }
}

fn check_hardware_authority(
    authority: &CapabilitySet,
    resources: NvmeHardwareResources,
) -> Result<(), NvmeError> {
    let io = CapabilityRights::io().with(CapabilityRight::Read);
    authority.check(CapabilityObject::PciDevice(resources.pci_device), io)?;
    authority.check(
        CapabilityObject::MmioRegion {
            base: resources.mmio_base,
            length: resources.mmio_length,
        },
        io.with(CapabilityRight::Write),
    )?;
    authority.check(
        CapabilityObject::DmaRegion(resources.dma_region),
        CapabilityRights::read_write_io(),
    )?;
    authority.check(
        CapabilityObject::IrqLine(resources.irq_line),
        CapabilityRights::io(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirage_block::BlockDevice;
    use mirage_cap::{Capability, CapabilityObject};

    fn resources() -> NvmeHardwareResources {
        NvmeHardwareResources::new(0x0108_0001, 0xfee0_0000, 0x4000, 7, 42)
    }

    fn full_authority() -> CapabilitySet {
        let resources = resources();
        CapabilitySet::from_capabilities(vec![
            Capability::new(
                CapabilityObject::PciDevice(resources.pci_device),
                CapabilityRights::read_write_io(),
            ),
            Capability::new(
                CapabilityObject::MmioRegion {
                    base: resources.mmio_base,
                    length: resources.mmio_length,
                },
                CapabilityRights::read_write_io(),
            ),
            Capability::new(
                CapabilityObject::DmaRegion(resources.dma_region),
                CapabilityRights::read_write_io(),
            ),
            Capability::new(
                CapabilityObject::IrqLine(resources.irq_line),
                CapabilityRights::io(),
            ),
        ])
    }

    fn namespace(id: u32, block_device_id: u64) -> NvmeNamespace {
        NvmeNamespace::new(
            NvmeNamespaceId::new(id),
            BlockDeviceId::new(block_device_id),
            BlockSize::new(512).unwrap(),
            SectorCount::new(8),
            false,
        )
        .unwrap()
    }

    fn controller_with_authority(authority: CapabilitySet) -> NvmeController {
        NvmeController::new(
            NvmeControllerId::new(1),
            resources(),
            authority,
            vec![namespace(1, 100), namespace(2, 200)],
        )
    }

    #[test]
    fn mock_identify_controller_reports_controller_data() {
        let mut controller = controller_with_authority(full_authority());

        let identify = controller.identify_controller().unwrap();

        assert_eq!(&identify.model_number[..11], b"Mirage NVMe");
        assert_eq!(identify.namespace_count, 2);
        assert!(identify.supports_write_cache);
        assert_eq!(controller.admin_queue().completions().len(), 1);
    }

    #[test]
    fn mock_namespace_discovery_reports_namespaces() {
        let mut controller = controller_with_authority(full_authority());

        let namespaces = controller.discover_namespaces().unwrap();

        assert_eq!(namespaces.len(), 2);
        assert_eq!(namespaces[0].id(), NvmeNamespaceId::new(1));
        assert_eq!(namespaces[1].block_device_id(), BlockDeviceId::new(200));
    }

    #[test]
    fn read_write_through_block_device_round_trips_data() {
        let controller = controller_with_authority(full_authority());
        let mut device = controller
            .into_block_device(NvmeNamespaceId::new(1))
            .unwrap();
        let range = BlockRange::new(Lba::new(2), SectorCount::new(1));
        let written = vec![0xa5; 512];
        let mut read = vec![0; 512];

        device.write_blocks(range, &written).unwrap();
        device.read_blocks(range, &mut read).unwrap();

        assert_eq!(read, written);
        assert_eq!(device.info().id, BlockDeviceId::new(100));
    }

    #[test]
    fn operation_rejection_without_required_capability() {
        let mut controller = controller_with_authority(CapabilitySet::from_capabilities(vec![
            Capability::new(
                CapabilityObject::PciDevice(resources().pci_device),
                CapabilityRights::read_write_io(),
            ),
            Capability::new(
                CapabilityObject::MmioRegion {
                    base: resources().mmio_base,
                    length: resources().mmio_length,
                },
                CapabilityRights::read_write_io(),
            ),
            Capability::new(
                CapabilityObject::DmaRegion(resources().dma_region),
                CapabilityRights::read_write_io(),
            ),
        ]));

        assert_eq!(
            controller.identify_controller(),
            Err(NvmeError::Capability(CapabilityError::Missing))
        );

        let mut device = controller
            .into_block_device(NvmeNamespaceId::new(1))
            .unwrap();
        let mut buffer = vec![0; 512];
        assert_eq!(
            device.read_blocks(
                BlockRange::new(Lba::new(0), SectorCount::new(1)),
                &mut buffer
            ),
            Err(BlockError::Io)
        );
    }
}
