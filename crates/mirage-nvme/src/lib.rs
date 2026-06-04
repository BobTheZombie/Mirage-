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
pub struct MockNvmeCommand {
    pub command_id: u16,
    pub opcode: NvmeOpcode,
    pub namespace_id: Option<NvmeNamespaceId>,
    pub lba: Lba,
    pub blocks: SectorCount,
}

impl MockNvmeCommand {
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
pub struct MockNvmeCompletion {
    pub command_id: u16,
    pub status: MockNvmeCompletionStatus,
}

impl MockNvmeCompletion {
    pub const fn success(command_id: u16) -> Self {
        Self {
            command_id,
            status: MockNvmeCompletionStatus::Success,
        }
    }

    pub const fn failed(command_id: u16, error: NvmeError) -> Self {
        Self {
            command_id,
            status: MockNvmeCompletionStatus::Failed(error),
        }
    }
}

/// Completion status for mock command execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MockNvmeCompletionStatus {
    Success,
    Failed(NvmeError),
}

/// Submission/completion queue pair owned by a supervised NVMe driver service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockNvmeQueuePair {
    id: u16,
    endpoint: EndpointId,
    submitted: Vec<MockNvmeCommand>,
    completed: Vec<MockNvmeCompletion>,
}

impl MockNvmeQueuePair {
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

    pub fn submit(&mut self, command: MockNvmeCommand) {
        self.submitted.push(command);
    }

    pub fn complete(&mut self, completion: MockNvmeCompletion) {
        self.completed.push(completion);
    }

    pub fn completions(&self) -> &[MockNvmeCompletion] {
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
pub struct MockNvmeController {
    id: NvmeControllerId,
    resources: NvmeHardwareResources,
    authority: CapabilitySet,
    identify: NvmeIdentifyData,
    admin_queue: MockNvmeQueuePair,
    io_queue: MockNvmeQueuePair,
    namespaces: Vec<NvmeNamespace>,
    state: BlockDeviceState,
}

impl MockNvmeController {
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
            admin_queue: MockNvmeQueuePair::new(0, EndpointId::new(id.get() << 32)),
            io_queue: MockNvmeQueuePair::new(1, EndpointId::new((id.get() << 32) | 1)),
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

    pub fn admin_queue(&self) -> &MockNvmeQueuePair {
        &self.admin_queue
    }

    pub fn io_queue(&self) -> &MockNvmeQueuePair {
        &self.io_queue
    }

    pub fn identify_controller(&mut self) -> Result<NvmeIdentifyData, NvmeError> {
        self.check_hardware_authority()?;
        let command = MockNvmeCommand::identify(1);
        self.admin_queue.submit(command.clone());
        // TODO: PRP/SGL mapping must translate identify buffers through DMA-safe memory.
        self.admin_queue
            .complete(MockNvmeCompletion::success(command.command_id));
        Ok(self.identify.clone())
    }

    pub fn discover_namespaces(&mut self) -> Result<&[NvmeNamespace], NvmeError> {
        self.check_hardware_authority()?;
        let command = MockNvmeCommand::identify(2);
        self.admin_queue.submit(command.clone());
        self.admin_queue
            .complete(MockNvmeCompletion::success(command.command_id));
        Ok(&self.namespaces)
    }

    pub fn into_block_device(
        self,
        namespace_id: NvmeNamespaceId,
    ) -> Result<MockNvmeBlockDevice, NvmeError> {
        let namespace = self
            .namespaces
            .into_iter()
            .find(|namespace| namespace.id() == namespace_id)
            .ok_or(NvmeError::NamespaceNotFound)?;
        Ok(MockNvmeBlockDevice::new(
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
pub struct MockNvmeBlockDevice {
    controller_id: NvmeControllerId,
    resources: NvmeHardwareResources,
    authority: CapabilitySet,
    namespace: NvmeNamespace,
    state: BlockDeviceState,
}

impl MockNvmeBlockDevice {
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

impl BlockDevice for MockNvmeBlockDevice {
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

#[cfg(feature = "hw-nvme")]
pub mod hw_nvme {
    use super::*;
    use mirage_pci::PciDevice;

    const DEFAULT_POLL_TICKS: u32 = 64;
    const NVME_BAR: usize = 0;

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub enum NvmeHwError {
        Capability(CapabilityError),
        NotNvmeDevice,
        MissingMmioBar,
        InvalidQueueDepth,
        QueueFull,
        QueueEmpty,
        Timeout { operation: &'static str, ticks: u32 },
        ControllerFault { status: NvmeStatus },
        NamespaceNotFound,
        BufferSizeMismatch,
        OutOfBounds,
        ReadOnly,
        Offline,
    }

    impl From<CapabilityError> for NvmeHwError {
        fn from(error: CapabilityError) -> Self {
            Self::Capability(error)
        }
    }

    impl From<BlockError> for NvmeHwError {
        fn from(error: BlockError) -> Self {
            match error {
                BlockError::BufferSizeMismatch => Self::BufferSizeMismatch,
                BlockError::OutOfBounds | BlockError::EmptyRange | BlockError::RangeOverflow => {
                    Self::OutOfBounds
                }
                BlockError::ReadOnly => Self::ReadOnly,
                BlockError::DeviceOffline | BlockError::DeviceFaulted => Self::Offline,
                BlockError::InvalidBlockSize
                | BlockError::QueueEmpty
                | BlockError::DeviceMismatch
                | BlockError::Io => Self::ControllerFault {
                    status: NvmeStatus::internal_error(),
                },
            }
        }
    }

    impl From<NvmeHwError> for BlockError {
        fn from(error: NvmeHwError) -> Self {
            match error {
                NvmeHwError::BufferSizeMismatch => Self::BufferSizeMismatch,
                NvmeHwError::OutOfBounds => Self::OutOfBounds,
                NvmeHwError::ReadOnly => Self::ReadOnly,
                NvmeHwError::Offline => Self::DeviceOffline,
                _ => Self::Io,
            }
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct NvmeRegisters {
        pub mmio_base: u64,
        pub mmio_length: u64,
        cap: u64,
        cc: u32,
        csts: u32,
        aqa: u32,
        asq: u64,
        acq: u64,
    }
    impl NvmeRegisters {
        pub const fn new(mmio_base: u64, mmio_length: u64) -> Self {
            Self {
                mmio_base,
                mmio_length,
                cap: 0,
                cc: 0,
                csts: 0,
                aqa: 0,
                asq: 0,
                acq: 0,
            }
        }
        pub const fn controller_enabled(&self) -> bool {
            (self.cc & 1) != 0
        }
        pub const fn ready(&self) -> bool {
            (self.csts & 1) != 0
        }
        fn set_enabled(&mut self, enabled: bool) {
            if enabled {
                self.cc |= 1;
                self.csts |= 1;
            } else {
                self.cc &= !1;
                self.csts &= !1;
            }
        }
        fn set_admin_queue(&mut self, depth: u16, submission: u64, completion: u64) {
            self.aqa =
                u32::from(depth.saturating_sub(1)) | (u32::from(depth.saturating_sub(1)) << 16);
            self.asq = submission;
            self.acq = completion;
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct NvmeStatus {
        pub status_code_type: u8,
        pub status_code: u8,
        pub phase: bool,
    }
    impl NvmeStatus {
        pub const fn success() -> Self {
            Self {
                status_code_type: 0,
                status_code: 0,
                phase: true,
            }
        }
        pub const fn internal_error() -> Self {
            Self {
                status_code_type: 0,
                status_code: 6,
                phase: true,
            }
        }
        pub const fn is_success(self) -> bool {
            self.status_code_type == 0 && self.status_code == 0
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct NvmeCommand {
        pub opcode: u8,
        pub command_id: u16,
        pub namespace_id: u32,
        pub prp1: u64,
        pub prp2: u64,
        pub cdw10: u32,
        pub cdw11: u32,
        pub cdw12: u32,
    }
    impl NvmeCommand {
        pub const IDENTIFY: u8 = 0x06;
        pub const FLUSH: u8 = 0x00;
        pub const WRITE: u8 = 0x01;
        pub const READ: u8 = 0x02;
        pub const fn identify(command_id: u16, cns: u32, prp1: u64) -> Self {
            Self {
                opcode: Self::IDENTIFY,
                command_id,
                namespace_id: 0,
                prp1,
                prp2: 0,
                cdw10: cns,
                cdw11: 0,
                cdw12: 0,
            }
        }
        pub const fn read_write(
            opcode: u8,
            command_id: u16,
            namespace_id: NvmeNamespaceId,
            lba: Lba,
            blocks: SectorCount,
            prp: NvmePrpList,
        ) -> Self {
            Self {
                opcode,
                command_id,
                namespace_id: namespace_id.get(),
                prp1: prp.first,
                prp2: prp.second,
                cdw10: lba.get() as u32,
                cdw11: (lba.get() >> 32) as u32,
                cdw12: (blocks.get().saturating_sub(1)) as u32,
            }
        }
        pub const fn flush(command_id: u16, namespace_id: NvmeNamespaceId) -> Self {
            Self {
                opcode: Self::FLUSH,
                command_id,
                namespace_id: namespace_id.get(),
                prp1: 0,
                prp2: 0,
                cdw10: 0,
                cdw11: 0,
                cdw12: 0,
            }
        }
        pub fn encode(&self) -> [u32; 16] {
            let mut d = [0u32; 16];
            d[0] = u32::from(self.opcode) | (u32::from(self.command_id) << 16);
            d[1] = self.namespace_id;
            d[6] = self.prp1 as u32;
            d[7] = (self.prp1 >> 32) as u32;
            d[8] = self.prp2 as u32;
            d[9] = (self.prp2 >> 32) as u32;
            d[10] = self.cdw10;
            d[11] = self.cdw11;
            d[12] = self.cdw12;
            d
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct NvmeCompletion {
        pub command_id: u16,
        pub status: NvmeStatus,
        pub result: u32,
    }
    impl NvmeCompletion {
        pub const fn success(command_id: u16) -> Self {
            Self {
                command_id,
                status: NvmeStatus::success(),
                result: 0,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct NvmePrpList {
        pub first: u64,
        pub second: u64,
        pub byte_len: usize,
    }
    impl NvmePrpList {
        pub const fn new(first: u64, second: u64, byte_len: usize) -> Self {
            Self {
                first,
                second,
                byte_len,
            }
        }
        pub const fn is_page_aligned(self) -> bool {
            (self.first & 0xfff) == 0 && (self.second & 0xfff) == 0
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct NvmeNamespaceInfo {
        pub id: NvmeNamespaceId,
        pub block_device_id: BlockDeviceId,
        pub block_size: BlockSize,
        pub sectors: SectorCount,
        pub read_only: bool,
        pub write_cache: bool,
    }
    impl NvmeNamespaceInfo {
        pub const fn info(self) -> BlockDeviceInfo {
            BlockDeviceInfo::new(
                self.block_device_id,
                self.block_size,
                self.sectors,
                self.read_only,
                self.write_cache,
            )
        }
        pub fn validate_read(self, range: BlockRange, buffer: &[u8]) -> Result<(), BlockError> {
            let expected = self.info().expected_buffer_len(range)?;
            if buffer.len() == expected {
                Ok(())
            } else {
                Err(BlockError::BufferSizeMismatch)
            }
        }
        pub fn validate_write(self, range: BlockRange, data: &[u8]) -> Result<(), BlockError> {
            if self.read_only {
                return Err(BlockError::ReadOnly);
            }
            let expected = self.info().expected_buffer_len(range)?;
            if data.len() == expected {
                Ok(())
            } else {
                Err(BlockError::BufferSizeMismatch)
            }
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct NvmeSubmissionQueue {
        entries: Vec<NvmeCommand>,
        head: usize,
        tail: usize,
        depth: usize,
    }
    impl NvmeSubmissionQueue {
        pub fn new(depth: usize) -> Result<Self, NvmeHwError> {
            if depth < 2 {
                return Err(NvmeHwError::InvalidQueueDepth);
            }
            Ok(Self {
                entries: Vec::new(),
                head: 0,
                tail: 0,
                depth,
            })
        }
        pub fn submit(&mut self, cmd: NvmeCommand) -> Result<usize, NvmeHwError> {
            if self.entries.len() >= self.depth - 1 {
                return Err(NvmeHwError::QueueFull);
            }
            let slot = self.tail;
            self.entries.push(cmd);
            self.tail = (self.tail + 1) % self.depth;
            Ok(slot)
        }
        pub const fn tail(&self) -> usize {
            self.tail
        }
    }
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct NvmeCompletionQueue {
        entries: Vec<NvmeCompletion>,
        head: usize,
        depth: usize,
        phase: bool,
    }
    impl NvmeCompletionQueue {
        pub fn new(depth: usize) -> Result<Self, NvmeHwError> {
            if depth < 2 {
                return Err(NvmeHwError::InvalidQueueDepth);
            }
            Ok(Self {
                entries: Vec::new(),
                head: 0,
                depth,
                phase: true,
            })
        }
        pub fn push(&mut self, c: NvmeCompletion) -> Result<(), NvmeHwError> {
            if self.entries.len() >= self.depth {
                return Err(NvmeHwError::QueueFull);
            }
            self.entries.push(c);
            Ok(())
        }
        pub fn pop(&mut self) -> Result<NvmeCompletion, NvmeHwError> {
            if self.entries.is_empty() {
                return Err(NvmeHwError::QueueEmpty);
            }
            let c = self.entries.remove(0);
            self.head = (self.head + 1) % self.depth;
            if self.head == 0 {
                self.phase = !self.phase;
            }
            Ok(c)
        }
        pub const fn head(&self) -> usize {
            self.head
        }
        pub const fn phase(&self) -> bool {
            self.phase
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct NvmeAdminQueue {
        pub sq: NvmeSubmissionQueue,
        pub cq: NvmeCompletionQueue,
        pub timeout_ticks: u32,
    }
    impl NvmeAdminQueue {
        pub fn new(depth: usize) -> Result<Self, NvmeHwError> {
            Ok(Self {
                sq: NvmeSubmissionQueue::new(depth)?,
                cq: NvmeCompletionQueue::new(depth)?,
                timeout_ticks: DEFAULT_POLL_TICKS,
            })
        }
    }
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct NvmeIoQueue {
        pub id: u16,
        pub sq: NvmeSubmissionQueue,
        pub cq: NvmeCompletionQueue,
        pub timeout_ticks: u32,
    }
    impl NvmeIoQueue {
        pub fn new(id: u16, depth: usize) -> Result<Self, NvmeHwError> {
            Ok(Self {
                id,
                sq: NvmeSubmissionQueue::new(depth)?,
                cq: NvmeCompletionQueue::new(depth)?,
                timeout_ticks: DEFAULT_POLL_TICKS,
            })
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct NvmeController {
        id: NvmeControllerId,
        resources: NvmeHardwareResources,
        authority: CapabilitySet,
        registers: NvmeRegisters,
        admin_queue: Option<NvmeAdminQueue>,
        io_queue: Option<NvmeIoQueue>,
        namespaces: Vec<NvmeNamespaceInfo>,
        next_command_id: u16,
    }
    impl NvmeController {
        pub fn from_pci_device(
            id: NvmeControllerId,
            device: &PciDevice,
            authority: CapabilitySet,
            dma_region: u64,
        ) -> Result<Self, NvmeHwError> {
            if !device.is_nvme() {
                return Err(NvmeHwError::NotNvmeDevice);
            }
            let bar = device.bar(NVME_BAR).ok_or(NvmeHwError::MissingMmioBar)?;
            let len = bar.length().unwrap_or(0x4000);
            let resources = NvmeHardwareResources::new(
                u64::from(device.vendor_id().get()) << 16 | u64::from(device.device_id().get()),
                bar.base(),
                len,
                dma_region,
                u16::from(device.header().interrupt_line()),
            );
            Self::map_registers(id, resources, authority)
        }
        pub fn map_registers(
            id: NvmeControllerId,
            resources: NvmeHardwareResources,
            authority: CapabilitySet,
        ) -> Result<Self, NvmeHwError> {
            check_hardware_authority(&authority, resources).map_err(|error| match error {
                NvmeError::Capability(cap) => NvmeHwError::Capability(cap),
                _ => NvmeHwError::ControllerFault {
                    status: NvmeStatus::internal_error(),
                },
            })?;
            Ok(Self {
                id,
                resources,
                authority,
                registers: NvmeRegisters::new(resources.mmio_base, resources.mmio_length),
                admin_queue: None,
                io_queue: None,
                namespaces: Vec::new(),
                next_command_id: 1,
            })
        }
        pub fn reset(&mut self) -> Result<(), NvmeHwError> {
            self.registers.set_enabled(false);
            self.poll_until("controller reset", |s| !s.registers.ready())?;
            self.registers.set_enabled(true);
            self.poll_until("controller ready", |s| s.registers.ready())
        }
        pub fn init_admin_queue(&mut self, depth: usize) -> Result<(), NvmeHwError> {
            let q = NvmeAdminQueue::new(depth)?;
            self.registers.set_admin_queue(
                depth as u16,
                self.resources.dma_region,
                self.resources.dma_region + 0x1000,
            );
            self.admin_queue = Some(q);
            Ok(())
        }
        pub fn identify_controller(&mut self) -> Result<NvmeIdentifyData, NvmeHwError> {
            let id = self.alloc_command_id();
            let q = self.admin_queue.as_mut().ok_or(NvmeHwError::QueueEmpty)?;
            q.sq.submit(NvmeCommand::identify(id, 1, self.resources.dma_region))?;
            q.cq.push(NvmeCompletion::success(id))?;
            Self::poll_completion(q, id, "identify controller")?;
            Ok(NvmeIdentifyData::mock(self.namespaces.len() as u32))
        }
        pub fn identify_namespaces(&mut self) -> Result<&[NvmeNamespaceInfo], NvmeHwError> {
            if self.namespaces.is_empty() {
                self.namespaces.push(NvmeNamespaceInfo {
                    id: NvmeNamespaceId::new(1),
                    block_device_id: BlockDeviceId::new(self.id.get()),
                    block_size: BlockSize::new(512).map_err(NvmeHwError::from)?,
                    sectors: SectorCount::new(1024),
                    read_only: false,
                    write_cache: true,
                });
            }
            Ok(&self.namespaces)
        }
        pub fn create_io_queue_pair(&mut self, id: u16, depth: usize) -> Result<(), NvmeHwError> {
            self.io_queue = Some(NvmeIoQueue::new(id, depth)?);
            Ok(())
        }
        pub fn submit_read(
            &mut self,
            namespace_id: NvmeNamespaceId,
            range: BlockRange,
            buffer: &mut [u8],
        ) -> Result<(), NvmeHwError> {
            let ns = self.namespace(namespace_id)?;
            ns.validate_read(range, buffer)?;
            let id = self.alloc_command_id();
            let cmd = NvmeCommand::read_write(
                NvmeCommand::READ,
                id,
                namespace_id,
                range.start(),
                range.count(),
                NvmePrpList::new(self.resources.dma_region, 0, buffer.len()),
            );
            self.submit_io(cmd, id, "read")
        }
        pub fn submit_write(
            &mut self,
            namespace_id: NvmeNamespaceId,
            range: BlockRange,
            data: &[u8],
        ) -> Result<(), NvmeHwError> {
            let ns = self.namespace(namespace_id)?;
            ns.validate_write(range, data)?;
            let id = self.alloc_command_id();
            let cmd = NvmeCommand::read_write(
                NvmeCommand::WRITE,
                id,
                namespace_id,
                range.start(),
                range.count(),
                NvmePrpList::new(self.resources.dma_region, 0, data.len()),
            );
            self.submit_io(cmd, id, "write")
        }
        pub fn flush(&mut self, namespace_id: NvmeNamespaceId) -> Result<(), NvmeHwError> {
            self.namespace(namespace_id)?;
            let id = self.alloc_command_id();
            self.submit_io(NvmeCommand::flush(id, namespace_id), id, "flush")
        }
        pub fn registers(&self) -> &NvmeRegisters {
            &self.registers
        }
        fn alloc_command_id(&mut self) -> u16 {
            let id = self.next_command_id;
            self.next_command_id = self.next_command_id.wrapping_add(1).max(1);
            id
        }
        fn namespace(&self, id: NvmeNamespaceId) -> Result<NvmeNamespaceInfo, NvmeHwError> {
            self.namespaces
                .iter()
                .copied()
                .find(|n| n.id == id)
                .ok_or(NvmeHwError::NamespaceNotFound)
        }
        fn submit_io(
            &mut self,
            cmd: NvmeCommand,
            command_id: u16,
            operation: &'static str,
        ) -> Result<(), NvmeHwError> {
            let q = self.io_queue.as_mut().ok_or(NvmeHwError::QueueEmpty)?;
            q.sq.submit(cmd)?;
            q.cq.push(NvmeCompletion::success(command_id))?;
            Self::poll_completion_io(q, command_id, operation)
        }
        fn poll_completion(
            q: &mut NvmeAdminQueue,
            command_id: u16,
            operation: &'static str,
        ) -> Result<NvmeCompletion, NvmeHwError> {
            for _ in 0..q.timeout_ticks {
                if let Ok(c) = q.cq.pop() {
                    if c.command_id == command_id && c.status.is_success() {
                        return Ok(c);
                    }
                    return Err(NvmeHwError::ControllerFault { status: c.status });
                }
            }
            Err(NvmeHwError::Timeout {
                operation,
                ticks: q.timeout_ticks,
            })
        }
        fn poll_completion_io(
            q: &mut NvmeIoQueue,
            command_id: u16,
            operation: &'static str,
        ) -> Result<(), NvmeHwError> {
            for _ in 0..q.timeout_ticks {
                if let Ok(c) = q.cq.pop() {
                    if c.command_id == command_id && c.status.is_success() {
                        return Ok(());
                    }
                    return Err(NvmeHwError::ControllerFault { status: c.status });
                }
            }
            Err(NvmeHwError::Timeout {
                operation,
                ticks: q.timeout_ticks,
            })
        }
        fn poll_until<F: Fn(&Self) -> bool>(
            &self,
            operation: &'static str,
            ready: F,
        ) -> Result<(), NvmeHwError> {
            for _ in 0..DEFAULT_POLL_TICKS {
                if ready(self) {
                    return Ok(());
                }
            }
            Err(NvmeHwError::Timeout {
                operation,
                ticks: DEFAULT_POLL_TICKS,
            })
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct RealNvmeBlockDevice {
        controller: NvmeController,
        namespace: NvmeNamespaceInfo,
        state: BlockDeviceState,
    }
    impl RealNvmeBlockDevice {
        pub const fn new(controller: NvmeController, namespace: NvmeNamespaceInfo) -> Self {
            Self {
                controller,
                namespace,
                state: BlockDeviceState::Online,
            }
        }
    }
    impl BlockDevice for RealNvmeBlockDevice {
        fn info(&self) -> BlockDeviceInfo {
            self.namespace.info()
        }
        fn state(&self) -> BlockDeviceState {
            self.state
        }
        fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError> {
            self.controller
                .submit_read(self.namespace.id, range, buffer)
                .map_err(BlockError::from)
        }
        fn write_blocks(&mut self, range: BlockRange, data: &[u8]) -> Result<(), BlockError> {
            self.controller
                .submit_write(self.namespace.id, range, data)
                .map_err(BlockError::from)
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            self.controller
                .flush(self.namespace.id)
                .map_err(BlockError::from)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn nvme_command_encoding_places_opcode_id_lba_and_nlb() {
            let c = NvmeCommand::read_write(
                NvmeCommand::READ,
                7,
                NvmeNamespaceId::new(9),
                Lba::new(0x1_0000_0002),
                SectorCount::new(4),
                NvmePrpList::new(0x2000, 0x3000, 512),
            );
            let e = c.encode();
            assert_eq!(e[0], 0x0007_0002);
            assert_eq!(e[1], 9);
            assert_eq!(e[10], 2);
            assert_eq!(e[11], 1);
            assert_eq!(e[12], 3);
        }
        #[test]
        fn nvme_queue_wraps_tail_and_completion_phase() {
            let mut sq = NvmeSubmissionQueue::new(4).unwrap();
            for id in 1..=3 {
                sq.submit(NvmeCommand::flush(id, NvmeNamespaceId::new(1)))
                    .unwrap();
            }
            assert_eq!(sq.tail(), 3);
            assert_eq!(
                sq.submit(NvmeCommand::flush(4, NvmeNamespaceId::new(1))),
                Err(NvmeHwError::QueueFull)
            );
            let mut cq = NvmeCompletionQueue::new(2).unwrap();
            cq.push(NvmeCompletion::success(1)).unwrap();
            cq.pop().unwrap();
            assert_eq!(cq.head(), 1);
            cq.push(NvmeCompletion::success(2)).unwrap();
            cq.pop().unwrap();
            assert_eq!(cq.head(), 0);
            assert!(!cq.phase());
        }
        #[test]
        fn nvme_bounds_validation_rejects_short_read_buffer() {
            let ns = NvmeNamespaceInfo {
                id: NvmeNamespaceId::new(1),
                block_device_id: BlockDeviceId::new(1),
                block_size: BlockSize::new(512).unwrap(),
                sectors: SectorCount::new(1),
                read_only: false,
                write_cache: true,
            };
            assert_eq!(
                ns.validate_read(
                    BlockRange::new(Lba::new(0), SectorCount::new(1)),
                    &mut [0u8; 8]
                ),
                Err(BlockError::BufferSizeMismatch)
            );
        }
    }
}

#[cfg(feature = "hw-nvme")]
pub use hw_nvme::*;

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

    fn controller_with_authority(authority: CapabilitySet) -> MockNvmeController {
        MockNvmeController::new(
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
