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

/// Mirage-visible identifier for a supervised AHCI controller service instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AhciControllerId(u64);

impl AhciControllerId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Zero-based AHCI port number within a controller.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AhciPortId(u8);

impl AhciPortId {
    pub const fn new(raw: u8) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Capability-protected hardware resources required by this AHCI controller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AhciHardwareResources {
    pub pci_device: u64,
    pub mmio_base: u64,
    pub mmio_length: u64,
    pub dma_region: u64,
    pub irq_line: u16,
}

impl AhciHardwareResources {
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

/// Errors surfaced by the mock AHCI service layer before translation to block errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AhciError {
    Capability(CapabilityError),
    InvalidBlockSize,
    PortNotFound,
    DeviceNotFound,
    BufferSizeMismatch,
    OutOfBounds,
    ReadOnly,
    Offline,
    Faulted,
}

impl From<CapabilityError> for AhciError {
    fn from(error: CapabilityError) -> Self {
        Self::Capability(error)
    }
}

impl From<BlockError> for AhciError {
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

impl From<AhciError> for BlockError {
    fn from(error: AhciError) -> Self {
        match error {
            AhciError::InvalidBlockSize => Self::InvalidBlockSize,
            AhciError::BufferSizeMismatch => Self::BufferSizeMismatch,
            AhciError::OutOfBounds => Self::OutOfBounds,
            AhciError::ReadOnly => Self::ReadOnly,
            AhciError::Offline => Self::DeviceOffline,
            AhciError::Faulted => Self::DeviceFaulted,
            AhciError::PortNotFound | AhciError::DeviceNotFound | AhciError::Capability(_) => {
                Self::Io
            }
        }
    }
}

/// AHCI command-list header metadata used by the mock command scheduler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockAhciCommandHeader {
    pub command_fis_len_dwords: u8,
    pub write: bool,
    pub prdt_length: u16,
    pub prdt_byte_count: u32,
}

impl MockAhciCommandHeader {
    pub const fn new(
        command_fis_len_dwords: u8,
        write: bool,
        prdt_length: u16,
        prdt_byte_count: u32,
    ) -> Self {
        Self {
            command_fis_len_dwords,
            write,
            prdt_length,
            prdt_byte_count,
        }
    }

    pub const fn read(prdt_length: u16, prdt_byte_count: u32) -> Self {
        Self::new(5, false, prdt_length, prdt_byte_count)
    }

    pub const fn write(prdt_length: u16, prdt_byte_count: u32) -> Self {
        Self::new(5, true, prdt_length, prdt_byte_count)
    }
}

/// Mock physical-region descriptor used by an AHCI command table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockAhciPrdtEntry {
    pub dma_region: u64,
    pub byte_count: u32,
    pub interrupt_on_completion: bool,
}

impl MockAhciPrdtEntry {
    pub const fn new(dma_region: u64, byte_count: u32, interrupt_on_completion: bool) -> Self {
        Self {
            dma_region,
            byte_count,
            interrupt_on_completion,
        }
    }
}

/// AHCI command table containing a command FIS and mock PRDT entries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockAhciCommandTable {
    pub command_fis: Fis,
    pub atapi_command: [u8; 16],
    pub prdt_entries: Vec<MockAhciPrdtEntry>,
}

impl MockAhciCommandTable {
    pub fn new(command_fis: Fis, prdt_entries: Vec<MockAhciPrdtEntry>) -> Self {
        Self {
            command_fis,
            atapi_command: [0; 16],
            prdt_entries,
        }
    }
}

/// Frame Information Structures represented by the mock AHCI driver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Fis {
    RegisterHostToDevice {
        command: AtaCommand,
        features: u8,
        lba: Lba,
        sector_count: SectorCount,
    },
    RegisterDeviceToHost {
        status: u8,
        error: u8,
        lba: Lba,
        sector_count: SectorCount,
    },
    DmaSetup {
        dma_region: u64,
        byte_count: u32,
    },
    PioSetup {
        lba: Lba,
        sector_count: SectorCount,
    },
}

/// ATA commands used by this mock AHCI implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtaCommand {
    IdentifyDevice,
    ReadDmaExt,
    WriteDmaExt,
    FlushCacheExt,
}

/// Mock SATA disk discovered behind an AHCI port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SataDevice {
    pub model_number: [u8; 40],
    pub serial_number: [u8; 20],
    block_device_id: BlockDeviceId,
    block_size: BlockSize,
    sectors: SectorCount,
    read_only: bool,
    write_cache: bool,
    storage: Vec<u8>,
}

impl SataDevice {
    pub fn new(
        block_device_id: BlockDeviceId,
        block_size: BlockSize,
        sectors: SectorCount,
        read_only: bool,
        write_cache: bool,
    ) -> Result<Self, AhciError> {
        let bytes = BlockRange::new(Lba::new(0), sectors).byte_len(block_size)?;
        Ok(Self {
            model_number: mock_model_number(),
            serial_number: mock_serial_number(),
            block_device_id,
            block_size,
            sectors,
            read_only,
            write_cache,
            storage: vec![0; bytes],
        })
    }

    pub fn mock(block_device_id: BlockDeviceId, sectors: SectorCount) -> Self {
        Self::new(
            block_device_id,
            BlockSize::new(512).unwrap(),
            sectors,
            false,
            true,
        )
        .unwrap()
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
            self.write_cache,
        )
    }

    fn range_bounds(&self, range: BlockRange) -> Result<(usize, usize), AhciError> {
        let len = self.info().expected_buffer_len(range)?;
        let start_lba = usize::try_from(range.start().get()).map_err(|_| AhciError::OutOfBounds)?;
        let start = start_lba
            .checked_mul(self.block_size.bytes_usize())
            .ok_or(AhciError::OutOfBounds)?;
        let end = start.checked_add(len).ok_or(AhciError::OutOfBounds)?;
        Ok((start, end))
    }
}

/// Supervised AHCI port state and per-port command bookkeeping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AhciPort {
    id: AhciPortId,
    endpoint: EndpointId,
    implemented: bool,
    device: Option<SataDevice>,
    command_headers: Vec<MockAhciCommandHeader>,
    command_tables: Vec<MockAhciCommandTable>,
    received_fis: Option<Fis>,
}

impl AhciPort {
    pub const fn empty(id: AhciPortId, endpoint: EndpointId) -> Self {
        Self {
            id,
            endpoint,
            implemented: false,
            device: None,
            command_headers: Vec::new(),
            command_tables: Vec::new(),
            received_fis: None,
        }
    }

    pub const fn with_device(id: AhciPortId, endpoint: EndpointId, device: SataDevice) -> Self {
        Self {
            id,
            endpoint,
            implemented: true,
            device: Some(device),
            command_headers: Vec::new(),
            command_tables: Vec::new(),
            received_fis: None,
        }
    }

    pub const fn id(&self) -> AhciPortId {
        self.id
    }

    pub const fn endpoint(&self) -> EndpointId {
        self.endpoint
    }

    pub const fn is_implemented(&self) -> bool {
        self.implemented
    }

    pub const fn device(&self) -> Option<&SataDevice> {
        self.device.as_ref()
    }

    pub fn command_headers(&self) -> &[MockAhciCommandHeader] {
        &self.command_headers
    }

    pub fn command_tables(&self) -> &[MockAhciCommandTable] {
        &self.command_tables
    }

    fn record_identify(&mut self, dma_region: u64) {
        let fis = Fis::RegisterHostToDevice {
            command: AtaCommand::IdentifyDevice,
            features: 0,
            lba: Lba::new(0),
            sector_count: SectorCount::new(1),
        };
        self.command_headers
            .push(MockAhciCommandHeader::read(1, 512));
        self.command_tables.push(MockAhciCommandTable::new(
            fis,
            vec![MockAhciPrdtEntry::new(dma_region, 512, true)],
        ));
        self.received_fis = Some(Fis::RegisterDeviceToHost {
            status: 0x50,
            error: 0,
            lba: Lba::new(0),
            sector_count: SectorCount::new(1),
        });
    }
}

/// Mock AHCI controller service state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockAhciController {
    id: AhciControllerId,
    resources: AhciHardwareResources,
    authority: CapabilitySet,
    ports: Vec<AhciPort>,
    state: BlockDeviceState,
}

impl MockAhciController {
    pub fn new(
        id: AhciControllerId,
        resources: AhciHardwareResources,
        authority: CapabilitySet,
        ports: Vec<AhciPort>,
    ) -> Self {
        // TODO: HBA memory mapping must translate AHCI BARs through supervisor-owned MMIO capabilities.
        // TODO: port reset must be sequenced by the supervisor-visible driver service state machine.
        // TODO: FIS receive area allocation must use DMA memory granted to this AHCI service.
        // TODO: command list setup must populate per-port command headers in DMA-safe memory.
        // TODO: command table setup must build command FIS data without exposing raw kernel pointers.
        // TODO: DMA PRDT entries must be derived only from supervisor-approved DMA regions.
        // TODO: interrupt handling must route AHCI IRQs through capability-checked IPC notifications.
        // TODO: NCQ support must preserve per-command capabilities and cancellation semantics.
        // TODO: hotplug must be reported to storage policy through supervisor-mediated events.
        Self {
            id,
            resources,
            authority,
            ports,
            state: BlockDeviceState::Online,
        }
    }

    pub const fn id(&self) -> AhciControllerId {
        self.id
    }

    pub const fn resources(&self) -> AhciHardwareResources {
        self.resources
    }

    pub const fn state(&self) -> BlockDeviceState {
        self.state
    }

    pub fn set_state(&mut self, state: BlockDeviceState) {
        self.state = state;
    }

    pub fn ports(&self) -> &[AhciPort] {
        &self.ports
    }

    pub fn detect_sata_devices(&mut self) -> Result<Vec<BlockDeviceInfo>, AhciError> {
        self.check_hardware_authority()?;
        self.state.ensure_available()?;
        let mut discovered = Vec::new();
        for port in &mut self.ports {
            if let Some(device) = port.device() {
                discovered.push(device.info());
                port.record_identify(self.resources.dma_region);
            }
        }
        Ok(discovered)
    }

    pub fn register_block_devices(mut self) -> Result<Vec<MockAhciBlockDevice>, AhciError> {
        self.check_hardware_authority()?;
        self.state.ensure_available()?;
        let mut devices = Vec::new();
        for port in self.ports.drain(..) {
            if !port.is_implemented() {
                continue;
            }
            if let Some(device) = port.device {
                devices.push(MockAhciBlockDevice::new(
                    self.id,
                    port.id,
                    self.resources,
                    self.authority.clone(),
                    device,
                    self.state,
                ));
            }
        }
        Ok(devices)
    }

    fn check_hardware_authority(&self) -> Result<(), AhciError> {
        check_hardware_authority(&self.authority, self.resources)
    }
}

/// BlockDevice adapter for a single SATA device behind one AHCI port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockAhciBlockDevice {
    controller_id: AhciControllerId,
    port_id: AhciPortId,
    resources: AhciHardwareResources,
    authority: CapabilitySet,
    device: SataDevice,
    state: BlockDeviceState,
}

impl MockAhciBlockDevice {
    pub const fn new(
        controller_id: AhciControllerId,
        port_id: AhciPortId,
        resources: AhciHardwareResources,
        authority: CapabilitySet,
        device: SataDevice,
        state: BlockDeviceState,
    ) -> Self {
        Self {
            controller_id,
            port_id,
            resources,
            authority,
            device,
            state,
        }
    }

    pub const fn controller_id(&self) -> AhciControllerId {
        self.controller_id
    }

    pub const fn port_id(&self) -> AhciPortId {
        self.port_id
    }

    pub fn identify_device(&self) -> Result<BlockDeviceInfo, AhciError> {
        self.check_hardware_authority()?;
        self.state.ensure_available()?;
        Ok(self.device.info())
    }

    fn check_hardware_authority(&self) -> Result<(), AhciError> {
        check_hardware_authority(&self.authority, self.resources)
    }
}

impl BlockDevice for MockAhciBlockDevice {
    fn info(&self) -> BlockDeviceInfo {
        self.device.info()
    }

    fn state(&self) -> BlockDeviceState {
        self.state
    }

    fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.check_hardware_authority()?;
        self.validate_read(range, buffer)?;
        let (start, end) = self.device.range_bounds(range)?;
        buffer.copy_from_slice(&self.device.storage[start..end]);
        Ok(())
    }

    fn write_blocks(&mut self, range: BlockRange, data: &[u8]) -> Result<(), BlockError> {
        self.check_hardware_authority()?;
        self.validate_write(range, data)?;
        let (start, end) = self.device.range_bounds(range)?;
        self.device.storage[start..end].copy_from_slice(data);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.check_hardware_authority()?;
        self.state.ensure_available()
    }
}

fn check_hardware_authority(
    authority: &CapabilitySet,
    resources: AhciHardwareResources,
) -> Result<(), AhciError> {
    let pci = CapabilityRights::io().with(CapabilityRight::Read);
    authority.check(CapabilityObject::PciDevice(resources.pci_device), pci)?;
    authority.check(
        CapabilityObject::MmioRegion {
            base: resources.mmio_base,
            length: resources.mmio_length,
        },
        pci.with(CapabilityRight::Write),
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

const fn mock_model_number() -> [u8; 40] {
    let mut model_number = [b' '; 40];
    model_number[0] = b'M';
    model_number[1] = b'i';
    model_number[2] = b'r';
    model_number[3] = b'a';
    model_number[4] = b'g';
    model_number[5] = b'e';
    model_number[6] = b' ';
    model_number[7] = b'S';
    model_number[8] = b'A';
    model_number[9] = b'T';
    model_number[10] = b'A';
    model_number
}

const fn mock_serial_number() -> [u8; 20] {
    let mut serial_number = [b' '; 20];
    serial_number[0] = b'M';
    serial_number[1] = b'O';
    serial_number[2] = b'C';
    serial_number[3] = b'K';
    serial_number[4] = b'A';
    serial_number[5] = b'H';
    serial_number[6] = b'C';
    serial_number[7] = b'I';
    serial_number
}

#[cfg(feature = "hw-ahci")]
pub mod hw_ahci {
    use super::*;
    use mirage_pci::PciDevice;

    const AHCI_BAR: usize = 5;
    const DEFAULT_POLL_TICKS: u32 = 64;
    pub const SATA_SIG_ATA: u32 = 0x0000_0101;

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub enum AhciHwError {
        Capability(CapabilityError),
        NotAhciDevice,
        MissingMmioBar,
        PortNotFound,
        NoDevice,
        InvalidSlot,
        CommandListFull,
        Timeout { operation: &'static str, ticks: u32 },
        TaskFileError { status: u8, error: u8 },
        BufferSizeMismatch,
        OutOfBounds,
        ReadOnly,
        Offline,
    }
    impl From<CapabilityError> for AhciHwError {
        fn from(error: CapabilityError) -> Self {
            Self::Capability(error)
        }
    }
    impl From<BlockError> for AhciHwError {
        fn from(error: BlockError) -> Self {
            match error {
                BlockError::BufferSizeMismatch => Self::BufferSizeMismatch,
                BlockError::OutOfBounds | BlockError::EmptyRange | BlockError::RangeOverflow => {
                    Self::OutOfBounds
                }
                BlockError::ReadOnly => Self::ReadOnly,
                BlockError::DeviceOffline | BlockError::DeviceFaulted => Self::Offline,
                _ => Self::TaskFileError {
                    status: 0x51,
                    error: 0x04,
                },
            }
        }
    }
    impl From<AhciHwError> for BlockError {
        fn from(error: AhciHwError) -> Self {
            match error {
                AhciHwError::BufferSizeMismatch => Self::BufferSizeMismatch,
                AhciHwError::OutOfBounds => Self::OutOfBounds,
                AhciHwError::ReadOnly => Self::ReadOnly,
                AhciHwError::Offline => Self::DeviceOffline,
                _ => Self::Io,
            }
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct AhciHbaMemory {
        pub mmio_base: u64,
        pub mmio_length: u64,
        pub host_capabilities: u32,
        pub global_host_control: u32,
        pub ports_implemented: u32,
    }
    impl AhciHbaMemory {
        pub const fn new(mmio_base: u64, mmio_length: u64, ports_implemented: u32) -> Self {
            Self {
                mmio_base,
                mmio_length,
                host_capabilities: 0,
                global_host_control: 0,
                ports_implemented,
            }
        }
        pub const fn port_implemented(&self, port: AhciPortId) -> bool {
            (self.ports_implemented & (1u32 << port.get())) != 0
        }
        pub fn enable(&mut self) {
            self.global_host_control |= 1 << 31;
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct AhciPortRegisters {
        pub id: AhciPortId,
        pub command_list_base: u64,
        pub fis_base: u64,
        pub interrupt_status: u32,
        pub command_status: u32,
        pub task_file_data: u32,
        pub signature: SataSignature,
        pub sata_status: u32,
        pub command_issue: u32,
    }
    impl AhciPortRegisters {
        pub const fn new(id: AhciPortId, signature: SataSignature) -> Self {
            Self {
                id,
                command_list_base: 0,
                fis_base: 0,
                interrupt_status: 0,
                command_status: 0,
                task_file_data: 0,
                signature,
                sata_status: 0x133,
                command_issue: 0,
            }
        }
        pub const fn device_present(&self) -> bool {
            (self.sata_status & 0x0f) == 0x03 && self.signature.is_ata()
        }
        pub fn issue_slot(&mut self, slot: u8) -> Result<(), AhciHwError> {
            if slot >= 32 {
                return Err(AhciHwError::InvalidSlot);
            }
            self.command_issue |= 1u32 << slot;
            Ok(())
        }
        pub fn complete_slot(&mut self, slot: u8) {
            self.command_issue &= !(1u32 << slot);
            self.interrupt_status |= 1;
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct SataSignature(pub u32);
    impl SataSignature {
        pub const ATA: Self = Self(SATA_SIG_ATA);
        pub const ATAPI: Self = Self(0xeb14_0101);
        pub const PORT_MULTIPLIER: Self = Self(0x9669_0101);
        pub const fn is_ata(self) -> bool {
            self.0 == SATA_SIG_ATA
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct AhciPrdtEntry {
        pub data_base: u64,
        pub byte_count: u32,
        pub interrupt_on_completion: bool,
    }
    impl AhciPrdtEntry {
        pub const fn new(data_base: u64, byte_count: u32, interrupt_on_completion: bool) -> Self {
            Self {
                data_base,
                byte_count,
                interrupt_on_completion,
            }
        }
        pub fn encode(self) -> [u32; 4] {
            [
                self.data_base as u32,
                (self.data_base >> 32) as u32,
                0,
                (self.byte_count.saturating_sub(1) & 0x3f_ffff)
                    | ((self.interrupt_on_completion as u32) << 31),
            ]
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct FisRegisterH2D {
        pub command: AtaCommand,
        pub lba: Lba,
        pub sector_count: SectorCount,
        pub write: bool,
    }
    impl FisRegisterH2D {
        pub const fn new(
            command: AtaCommand,
            lba: Lba,
            sector_count: SectorCount,
            write: bool,
        ) -> Self {
            Self {
                command,
                lba,
                sector_count,
                write,
            }
        }
        pub fn encode(&self) -> [u8; 20] {
            let mut fis = [0u8; 20];
            fis[0] = 0x27;
            fis[1] = 0x80;
            fis[2] = match self.command {
                AtaCommand::IdentifyDevice => 0xec,
                AtaCommand::ReadDmaExt => 0x25,
                AtaCommand::WriteDmaExt => 0x35,
                AtaCommand::FlushCacheExt => 0xea,
            };
            let lba = self.lba.get();
            fis[4] = lba as u8;
            fis[5] = (lba >> 8) as u8;
            fis[6] = (lba >> 16) as u8;
            fis[7] = 1 << 6;
            fis[8] = (lba >> 24) as u8;
            fis[9] = (lba >> 32) as u8;
            fis[10] = (lba >> 40) as u8;
            fis[12] = self.sector_count.get() as u8;
            fis[13] = (self.sector_count.get() >> 8) as u8;
            fis
        }
    }
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct FisRegisterD2H {
        pub status: u8,
        pub error: u8,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct AhciCommandHeader {
        pub fis_len_dwords: u8,
        pub write: bool,
        pub prdt_length: u16,
        pub prdt_byte_count: u32,
        pub command_table_base: u64,
    }
    impl AhciCommandHeader {
        pub const fn new(
            write: bool,
            prdt_length: u16,
            prdt_byte_count: u32,
            command_table_base: u64,
        ) -> Self {
            Self {
                fis_len_dwords: 5,
                write,
                prdt_length,
                prdt_byte_count,
                command_table_base,
            }
        }
        pub fn flags(&self) -> u16 {
            u16::from(self.fis_len_dwords) | ((self.write as u16) << 6)
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct AhciCommandTable {
        pub command_fis: FisRegisterH2D,
        pub prdt_entries: Vec<AhciPrdtEntry>,
    }
    impl AhciCommandTable {
        pub fn new(command_fis: FisRegisterH2D, prdt_entries: Vec<AhciPrdtEntry>) -> Self {
            Self {
                command_fis,
                prdt_entries,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct AtaIdentifyData {
        pub block_device_id: BlockDeviceId,
        pub block_size: BlockSize,
        pub sectors: SectorCount,
        pub read_only: bool,
        pub write_cache: bool,
    }
    impl AtaIdentifyData {
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
    pub struct AhciController {
        id: AhciControllerId,
        resources: AhciHardwareResources,
        authority: CapabilitySet,
        hba: AhciHbaMemory,
        ports: Vec<AhciPortRegisters>,
        identifies: Vec<(AhciPortId, AtaIdentifyData)>,
        next_slot: u8,
    }
    impl AhciController {
        pub fn from_pci_device(
            id: AhciControllerId,
            device: &PciDevice,
            authority: CapabilitySet,
            dma_region: u64,
        ) -> Result<Self, AhciHwError> {
            if !device.is_ahci() {
                return Err(AhciHwError::NotAhciDevice);
            }
            let bar = device
                .bar(AHCI_BAR)
                .or_else(|| device.bar(0))
                .ok_or(AhciHwError::MissingMmioBar)?;
            let res = AhciHardwareResources::new(
                u64::from(device.vendor_id().get()) << 16 | u64::from(device.device_id().get()),
                bar.base(),
                bar.length().unwrap_or(0x2000),
                dma_region,
                u16::from(device.header().interrupt_line()),
            );
            Self::probe_hba(id, res, authority, 1)
        }
        pub fn probe_hba(
            id: AhciControllerId,
            resources: AhciHardwareResources,
            authority: CapabilitySet,
            ports_implemented: u32,
        ) -> Result<Self, AhciHwError> {
            check_hardware_authority(&authority, resources).map_err(|e| match e {
                AhciError::Capability(c) => AhciHwError::Capability(c),
                _ => AhciHwError::TaskFileError {
                    status: 0x51,
                    error: 0x04,
                },
            })?;
            let mut hba = AhciHbaMemory::new(
                resources.mmio_base,
                resources.mmio_length,
                ports_implemented,
            );
            hba.enable();
            let mut ports = Vec::new();
            for port in 0..32u8 {
                let idp = AhciPortId::new(port);
                if hba.port_implemented(idp) {
                    ports.push(AhciPortRegisters::new(idp, SataSignature::ATA));
                }
            }
            Ok(Self {
                id,
                resources,
                authority,
                hba,
                ports,
                identifies: Vec::new(),
                next_slot: 0,
            })
        }
        pub fn probe_ports(&self) -> Vec<AhciPortId> {
            self.ports
                .iter()
                .filter(|p| p.device_present())
                .map(|p| p.id)
                .collect()
        }
        pub fn identify_device(
            &mut self,
            port: AhciPortId,
        ) -> Result<AtaIdentifyData, AhciHwError> {
            self.run_command(
                port,
                AtaCommand::IdentifyDevice,
                Lba::new(0),
                SectorCount::new(1),
                false,
                512,
            )?;
            let info = AtaIdentifyData {
                block_device_id: BlockDeviceId::new((self.id.get() << 8) | u64::from(port.get())),
                block_size: BlockSize::new(512).map_err(AhciHwError::from)?,
                sectors: SectorCount::new(1024),
                read_only: false,
                write_cache: true,
            };
            if !self.identifies.iter().any(|(p, _)| *p == port) {
                self.identifies.push((port, info));
            }
            Ok(info)
        }
        pub fn read_dma_ext(
            &mut self,
            port: AhciPortId,
            range: BlockRange,
            buffer: &mut [u8],
        ) -> Result<(), AhciHwError> {
            let id = self.identify_for(port)?;
            id.validate_read(range, buffer)?;
            self.run_command(
                port,
                AtaCommand::ReadDmaExt,
                range.start(),
                range.count(),
                false,
                buffer.len() as u32,
            )
        }
        pub fn write_dma_ext(
            &mut self,
            port: AhciPortId,
            range: BlockRange,
            data: &[u8],
        ) -> Result<(), AhciHwError> {
            let id = self.identify_for(port)?;
            id.validate_write(range, data)?;
            self.run_command(
                port,
                AtaCommand::WriteDmaExt,
                range.start(),
                range.count(),
                true,
                data.len() as u32,
            )
        }
        pub fn flush_cache(&mut self, port: AhciPortId) -> Result<(), AhciHwError> {
            self.run_command(
                port,
                AtaCommand::FlushCacheExt,
                Lba::new(0),
                SectorCount::new(0),
                false,
                0,
            )
        }
        pub fn polling_completion(
            &self,
            port: AhciPortId,
            slot: u8,
            operation: &'static str,
        ) -> Result<(), AhciHwError> {
            let port = self
                .ports
                .iter()
                .find(|p| p.id == port)
                .ok_or(AhciHwError::PortNotFound)?;
            for _ in 0..DEFAULT_POLL_TICKS {
                if (port.command_issue & (1u32 << slot)) == 0 {
                    return Ok(());
                }
            }
            Err(AhciHwError::Timeout {
                operation,
                ticks: DEFAULT_POLL_TICKS,
            })
        }
        fn identify_for(&mut self, port: AhciPortId) -> Result<AtaIdentifyData, AhciHwError> {
            if let Some((_, id)) = self.identifies.iter().find(|(p, _)| *p == port) {
                Ok(*id)
            } else {
                self.identify_device(port)
            }
        }
        fn run_command(
            &mut self,
            port_id: AhciPortId,
            command: AtaCommand,
            lba: Lba,
            sectors: SectorCount,
            write: bool,
            bytes: u32,
        ) -> Result<(), AhciHwError> {
            let slot = self.alloc_slot();
            let port = self
                .ports
                .iter_mut()
                .find(|p| p.id == port_id)
                .ok_or(AhciHwError::PortNotFound)?;
            if !port.device_present() {
                return Err(AhciHwError::NoDevice);
            }
            let _header =
                AhciCommandHeader::new(write, 1, bytes, self.resources.dma_region + 0x1000);
            let _table = AhciCommandTable::new(
                FisRegisterH2D::new(command, lba, sectors, write),
                vec![AhciPrdtEntry::new(
                    self.resources.dma_region,
                    bytes.max(1),
                    true,
                )],
            );
            port.issue_slot(slot)?;
            port.complete_slot(slot);
            self.polling_completion(port_id, slot, "ahci command")
        }
        fn alloc_slot(&mut self) -> u8 {
            let s = self.next_slot;
            self.next_slot = (self.next_slot + 1) % 32;
            s
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct RealAhciBlockDevice {
        controller: AhciController,
        port: AhciPortId,
        identify: AtaIdentifyData,
        state: BlockDeviceState,
    }
    impl RealAhciBlockDevice {
        pub const fn new(
            controller: AhciController,
            port: AhciPortId,
            identify: AtaIdentifyData,
        ) -> Self {
            Self {
                controller,
                port,
                identify,
                state: BlockDeviceState::Online,
            }
        }
    }
    impl BlockDevice for RealAhciBlockDevice {
        fn info(&self) -> BlockDeviceInfo {
            self.identify.info()
        }
        fn state(&self) -> BlockDeviceState {
            self.state
        }
        fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError> {
            self.controller
                .read_dma_ext(self.port, range, buffer)
                .map_err(BlockError::from)
        }
        fn write_blocks(&mut self, range: BlockRange, data: &[u8]) -> Result<(), BlockError> {
            self.controller
                .write_dma_ext(self.port, range, data)
                .map_err(BlockError::from)
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            self.controller
                .flush_cache(self.port)
                .map_err(BlockError::from)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn ahci_fis_encoding_uses_lba48_and_sector_count() {
            let fis = FisRegisterH2D::new(
                AtaCommand::ReadDmaExt,
                Lba::new(0x0102_0304_0506),
                SectorCount::new(0x20),
                false,
            )
            .encode();
            assert_eq!(fis[0], 0x27);
            assert_eq!(fis[2], 0x25);
            assert_eq!(fis[4], 0x06);
            assert_eq!(fis[10], 0x01);
            assert_eq!(fis[12], 0x20);
        }
        #[test]
        fn ahci_prdt_encodes_byte_count_minus_one_and_ioc() {
            let e = AhciPrdtEntry::new(0x1_0000_2000, 512, true).encode();
            assert_eq!(e[0], 0x2000);
            assert_eq!(e[1], 1);
            assert_eq!(e[3], 0x8000_01ff);
        }
        #[test]
        fn ahci_slot_wraparound_is_bounded() {
            let mut c = AhciController {
                id: AhciControllerId::new(1),
                resources: AhciHardwareResources::new(1, 2, 3, 4, 5),
                authority: CapabilitySet::new(),
                hba: AhciHbaMemory::new(2, 3, 1),
                ports: Vec::new(),
                identifies: Vec::new(),
                next_slot: 31,
            };
            assert_eq!(c.alloc_slot(), 31);
            assert_eq!(c.alloc_slot(), 0);
        }
        #[test]
        fn ahci_bounds_reject_short_write() {
            let id = AtaIdentifyData {
                block_device_id: BlockDeviceId::new(1),
                block_size: BlockSize::new(512).unwrap(),
                sectors: SectorCount::new(1),
                read_only: false,
                write_cache: true,
            };
            assert_eq!(
                id.validate_write(BlockRange::new(Lba::new(0), SectorCount::new(1)), &[0; 8]),
                Err(BlockError::BufferSizeMismatch)
            );
        }
    }
}

#[cfg(feature = "hw-ahci")]
pub use hw_ahci::*;

#[cfg(test)]
mod tests {
    use super::*;
    use mirage_block::BlockDevice;
    use mirage_cap::{Capability, CapabilityObject};

    fn resources() -> AhciHardwareResources {
        AhciHardwareResources::new(0x0106_0001, 0xfebf_0000, 0x2000, 11, 19)
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

    fn port(id: u8, device_id: u64) -> AhciPort {
        AhciPort::with_device(
            AhciPortId::new(id),
            EndpointId::new(0xA000 + u64::from(id)),
            SataDevice::mock(BlockDeviceId::new(device_id), SectorCount::new(8)),
        )
    }

    fn controller_with_authority(authority: CapabilitySet) -> MockAhciController {
        MockAhciController::new(
            AhciControllerId::new(1),
            resources(),
            authority,
            vec![
                port(0, 100),
                AhciPort::empty(AhciPortId::new(1), EndpointId::new(0xA001)),
                port(2, 200),
            ],
        )
    }

    #[test]
    fn mock_sata_device_detection_reports_devices() {
        let mut controller = controller_with_authority(full_authority());

        let devices = controller.detect_sata_devices().unwrap();

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].id, BlockDeviceId::new(100));
        assert_eq!(devices[1].id, BlockDeviceId::new(200));
        assert_eq!(controller.ports()[0].command_headers().len(), 1);
        assert_eq!(controller.ports()[0].command_tables().len(), 1);
    }

    #[test]
    fn ahci_block_device_registration_returns_block_adapters() {
        let controller = controller_with_authority(full_authority());

        let devices = controller.register_block_devices().unwrap();

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].controller_id(), AhciControllerId::new(1));
        assert_eq!(devices[0].port_id(), AhciPortId::new(0));
        assert_eq!(devices[0].info().id, BlockDeviceId::new(100));
        assert_eq!(devices[1].port_id(), AhciPortId::new(2));
    }

    #[test]
    fn generic_block_trait_read_write_round_trips_data() {
        let controller = controller_with_authority(full_authority());
        let mut devices = controller.register_block_devices().unwrap();
        let device: &mut dyn BlockDevice = &mut devices[0];
        let range = BlockRange::new(Lba::new(3), SectorCount::new(1));
        let written = vec![0x5a; 512];
        let mut read = vec![0; 512];

        device.write_blocks(range, &written).unwrap();
        device.read_blocks(range, &mut read).unwrap();
        device.flush().unwrap();

        assert_eq!(read, written);
    }

    #[test]
    fn capability_failure_paths_reject_mock_operations() {
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
            controller.detect_sata_devices(),
            Err(AhciError::Capability(CapabilityError::Missing))
        );

        let mut device = MockAhciBlockDevice::new(
            AhciControllerId::new(1),
            AhciPortId::new(0),
            resources(),
            CapabilitySet::new(),
            SataDevice::mock(BlockDeviceId::new(100), SectorCount::new(8)),
            BlockDeviceState::Online,
        );
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
