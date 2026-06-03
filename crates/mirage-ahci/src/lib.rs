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
pub struct AhciCommandHeader {
    pub command_fis_len_dwords: u8,
    pub write: bool,
    pub prdt_length: u16,
    pub prdt_byte_count: u32,
}

impl AhciCommandHeader {
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
pub struct AhciPrdtEntry {
    pub dma_region: u64,
    pub byte_count: u32,
    pub interrupt_on_completion: bool,
}

impl AhciPrdtEntry {
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
pub struct AhciCommandTable {
    pub command_fis: Fis,
    pub atapi_command: [u8; 16],
    pub prdt_entries: Vec<AhciPrdtEntry>,
}

impl AhciCommandTable {
    pub fn new(command_fis: Fis, prdt_entries: Vec<AhciPrdtEntry>) -> Self {
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
    command_headers: Vec<AhciCommandHeader>,
    command_tables: Vec<AhciCommandTable>,
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

    pub fn command_headers(&self) -> &[AhciCommandHeader] {
        &self.command_headers
    }

    pub fn command_tables(&self) -> &[AhciCommandTable] {
        &self.command_tables
    }

    fn record_identify(&mut self, dma_region: u64) {
        let fis = Fis::RegisterHostToDevice {
            command: AtaCommand::IdentifyDevice,
            features: 0,
            lba: Lba::new(0),
            sector_count: SectorCount::new(1),
        };
        self.command_headers.push(AhciCommandHeader::read(1, 512));
        self.command_tables.push(AhciCommandTable::new(
            fis,
            vec![AhciPrdtEntry::new(dma_region, 512, true)],
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
pub struct AhciController {
    id: AhciControllerId,
    resources: AhciHardwareResources,
    authority: CapabilitySet,
    ports: Vec<AhciPort>,
    state: BlockDeviceState,
}

impl AhciController {
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

    pub fn register_block_devices(mut self) -> Result<Vec<AhciBlockDevice>, AhciError> {
        self.check_hardware_authority()?;
        self.state.ensure_available()?;
        let mut devices = Vec::new();
        for port in self.ports.drain(..) {
            if !port.is_implemented() {
                continue;
            }
            if let Some(device) = port.device {
                devices.push(AhciBlockDevice::new(
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
pub struct AhciBlockDevice {
    controller_id: AhciControllerId,
    port_id: AhciPortId,
    resources: AhciHardwareResources,
    authority: CapabilitySet,
    device: SataDevice,
    state: BlockDeviceState,
}

impl AhciBlockDevice {
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

impl BlockDevice for AhciBlockDevice {
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

    fn controller_with_authority(authority: CapabilitySet) -> AhciController {
        AhciController::new(
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

        let mut device = AhciBlockDevice::new(
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
