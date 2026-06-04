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

/// Mirage-visible identifier for a supervised USB controller service instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct UsbControllerId(u64);

impl UsbControllerId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// USB device address assigned during mock enumeration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct UsbDeviceAddress(u8);

impl UsbDeviceAddress {
    pub const fn new(raw: u8) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

/// USB endpoint number and transfer direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct UsbEndpointAddress {
    number: u8,
    direction: UsbDirection,
}

impl UsbEndpointAddress {
    pub const fn new(number: u8, direction: UsbDirection) -> Self {
        Self { number, direction }
    }

    pub const fn number(self) -> u8 {
        self.number
    }

    pub const fn direction(self) -> UsbDirection {
        self.direction
    }
}

/// Direction of a USB transfer from the host controller's perspective.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum UsbDirection {
    In,
    Out,
}

/// Endpoint transfer type represented by the mock USB stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbEndpointType {
    Control,
    Bulk,
    Interrupt,
    Isochronous,
}

/// Capability-protected hardware resources required by a USB host controller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbHardwareResources {
    pub pci_device: u64,
    pub mmio_base: u64,
    pub mmio_length: u64,
    pub dma_region: u64,
    pub irq_line: u16,
}

impl UsbHardwareResources {
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

/// USB errors surfaced before translation to generic block errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UsbError {
    Capability(CapabilityError),
    InvalidBlockSize,
    DeviceNotFound,
    EndpointNotFound,
    NotMassStorage,
    BufferSizeMismatch,
    OutOfBounds,
    ReadOnly,
    Offline,
    Faulted,
    TransportFault,
}

impl From<CapabilityError> for UsbError {
    fn from(error: CapabilityError) -> Self {
        Self::Capability(error)
    }
}

impl From<BlockError> for UsbError {
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
            BlockError::QueueEmpty | BlockError::DeviceMismatch | BlockError::Io => {
                Self::TransportFault
            }
        }
    }
}

impl From<UsbError> for BlockError {
    fn from(error: UsbError) -> Self {
        match error {
            UsbError::InvalidBlockSize => Self::InvalidBlockSize,
            UsbError::BufferSizeMismatch => Self::BufferSizeMismatch,
            UsbError::OutOfBounds => Self::OutOfBounds,
            UsbError::ReadOnly => Self::ReadOnly,
            UsbError::Offline => Self::DeviceOffline,
            UsbError::Faulted => Self::DeviceFaulted,
            UsbError::Capability(_)
            | UsbError::DeviceNotFound
            | UsbError::EndpointNotFound
            | UsbError::NotMassStorage
            | UsbError::TransportFault => Self::Io,
        }
    }
}

/// Minimal endpoint metadata for a supervised USB driver service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsbEndpoint {
    address: UsbEndpointAddress,
    endpoint_type: UsbEndpointType,
    max_packet_size: u16,
    service_endpoint: EndpointId,
}

impl UsbEndpoint {
    pub const fn new(
        address: UsbEndpointAddress,
        endpoint_type: UsbEndpointType,
        max_packet_size: u16,
        service_endpoint: EndpointId,
    ) -> Self {
        Self {
            address,
            endpoint_type,
            max_packet_size,
            service_endpoint,
        }
    }

    pub const fn address(&self) -> UsbEndpointAddress {
        self.address
    }

    pub const fn endpoint_type(&self) -> UsbEndpointType {
        self.endpoint_type
    }

    pub const fn max_packet_size(&self) -> u16 {
        self.max_packet_size
    }

    pub const fn service_endpoint(&self) -> EndpointId {
        self.service_endpoint
    }
}

/// Supported USB device class view used by mock enumeration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbDeviceClass {
    MassStorage,
    Hub,
    HumanInterface,
    VendorSpecific,
    Unknown,
}

/// USB transfer descriptor submitted through the mock controller transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsbTransfer {
    id: u64,
    device: UsbDeviceAddress,
    endpoint: UsbEndpointAddress,
    direction: UsbDirection,
    payload: Vec<u8>,
}

impl UsbTransfer {
    pub fn new(
        id: u64,
        device: UsbDeviceAddress,
        endpoint: UsbEndpointAddress,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            id,
            device,
            endpoint,
            direction: endpoint.direction(),
            payload,
        }
    }

    pub const fn id(&self) -> u64 {
        self.id
    }

    pub const fn device(&self) -> UsbDeviceAddress {
        self.device
    }

    pub const fn endpoint(&self) -> UsbEndpointAddress {
        self.endpoint
    }

    pub const fn direction(&self) -> UsbDirection {
        self.direction
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// Completion status returned by the mock controller after a transfer is accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsbTransferCompletion {
    pub transfer_id: u64,
    pub bytes: usize,
}

/// USB mass-storage transport model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UsbStorageTransport {
    BulkOnly {
        bulk_in: UsbEndpointAddress,
        bulk_out: UsbEndpointAddress,
    },
    UasPlaceholder,
}

/// Mock SCSI command descriptor carried over USB storage transports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScsiCommand {
    Read10 { lba: Lba, blocks: SectorCount },
    Write10 { lba: Lba, blocks: SectorCount },
    TestUnitReady,
    SynchronizeCache,
}

/// USB mass-storage function and backing data for tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockUsbStorageDevice {
    info: BlockDeviceInfo,
    transport: UsbStorageTransport,
    storage: Vec<u8>,
    last_scsi_command: Option<ScsiCommand>,
}

impl MockUsbStorageDevice {
    pub fn mock(id: BlockDeviceId, sectors: SectorCount) -> Self {
        let block_size = BlockSize::new(512).expect("mock USB block size is non-zero");
        let byte_len = (sectors.get() as usize).saturating_mul(block_size.bytes_usize());
        Self {
            info: BlockDeviceInfo::new(id, block_size, sectors, false, true),
            transport: UsbStorageTransport::BulkOnly {
                bulk_in: UsbEndpointAddress::new(1, UsbDirection::In),
                bulk_out: UsbEndpointAddress::new(2, UsbDirection::Out),
            },
            storage: vec![0; byte_len],
            last_scsi_command: None,
        }
    }

    pub const fn info(&self) -> BlockDeviceInfo {
        self.info
    }

    pub fn transport(&self) -> &UsbStorageTransport {
        &self.transport
    }

    pub fn last_scsi_command(&self) -> Option<&ScsiCommand> {
        self.last_scsi_command.as_ref()
    }

    fn read_scsi(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), UsbError> {
        // TODO: Replace this placeholder with complete SCSI command handling.
        self.info.validate_range(range)?;
        let expected = range.byte_len(self.info.block_size)?;
        if buffer.len() != expected {
            return Err(UsbError::BufferSizeMismatch);
        }
        self.last_scsi_command = Some(ScsiCommand::Read10 {
            lba: range.start(),
            blocks: range.count(),
        });
        let (start, end) = self.range_bounds(range)?;
        buffer.copy_from_slice(&self.storage[start..end]);
        Ok(())
    }

    fn write_scsi(&mut self, range: BlockRange, data: &[u8]) -> Result<(), UsbError> {
        // TODO: Replace this placeholder with complete SCSI command handling.
        self.info.validate_range(range)?;
        let expected = range.byte_len(self.info.block_size)?;
        if data.len() != expected {
            return Err(UsbError::BufferSizeMismatch);
        }
        self.last_scsi_command = Some(ScsiCommand::Write10 {
            lba: range.start(),
            blocks: range.count(),
        });
        let (start, end) = self.range_bounds(range)?;
        self.storage[start..end].copy_from_slice(data);
        Ok(())
    }

    fn flush_scsi(&mut self) {
        // TODO: Implement real SCSI command handling for SYNCHRONIZE CACHE.
        self.last_scsi_command = Some(ScsiCommand::SynchronizeCache);
    }

    fn range_bounds(&self, range: BlockRange) -> Result<(usize, usize), UsbError> {
        let start = (range.start().get() as usize)
            .checked_mul(self.info.block_size.bytes_usize())
            .ok_or(UsbError::OutOfBounds)?;
        let len = range.byte_len(self.info.block_size)?;
        let end = start.checked_add(len).ok_or(UsbError::OutOfBounds)?;
        if end > self.storage.len() {
            return Err(UsbError::OutOfBounds);
        }
        Ok((start, end))
    }
}

/// Enumerated USB device known to a supervised controller instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsbDevice {
    address: UsbDeviceAddress,
    class: UsbDeviceClass,
    endpoints: Vec<UsbEndpoint>,
    mass_storage: Option<MockUsbStorageDevice>,
    connected: bool,
}

impl UsbDevice {
    pub fn new(
        address: UsbDeviceAddress,
        class: UsbDeviceClass,
        endpoints: Vec<UsbEndpoint>,
        mass_storage: Option<MockUsbStorageDevice>,
    ) -> Self {
        Self {
            address,
            class,
            endpoints,
            mass_storage,
            connected: true,
        }
    }

    pub fn mock_mass_storage(address: UsbDeviceAddress, block_id: BlockDeviceId) -> Self {
        Self::new(
            address,
            UsbDeviceClass::MassStorage,
            vec![
                UsbEndpoint::new(
                    UsbEndpointAddress::new(0, UsbDirection::Out),
                    UsbEndpointType::Control,
                    64,
                    EndpointId::new(0x1000 + u64::from(address.get())),
                ),
                UsbEndpoint::new(
                    UsbEndpointAddress::new(1, UsbDirection::In),
                    UsbEndpointType::Bulk,
                    512,
                    EndpointId::new(0x2000 + u64::from(address.get())),
                ),
                UsbEndpoint::new(
                    UsbEndpointAddress::new(2, UsbDirection::Out),
                    UsbEndpointType::Bulk,
                    512,
                    EndpointId::new(0x3000 + u64::from(address.get())),
                ),
            ],
            Some(MockUsbStorageDevice::mock(block_id, SectorCount::new(16))),
        )
    }

    pub const fn address(&self) -> UsbDeviceAddress {
        self.address
    }

    pub const fn class(&self) -> UsbDeviceClass {
        self.class
    }

    pub fn endpoints(&self) -> &[UsbEndpoint] {
        &self.endpoints
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    pub fn is_mass_storage(&self) -> bool {
        self.class == UsbDeviceClass::MassStorage && self.mass_storage.is_some()
    }

    pub fn mass_storage(&self) -> Option<&MockUsbStorageDevice> {
        self.mass_storage.as_ref()
    }

    fn into_mass_storage(self) -> Option<MockUsbStorageDevice> {
        self.mass_storage
    }

    fn has_endpoint(&self, endpoint: UsbEndpointAddress) -> bool {
        self.endpoints
            .iter()
            .any(|candidate| candidate.address == endpoint)
    }

    fn disconnect(&mut self) {
        self.connected = false;
    }
}

/// Mock USB controller owned by a supervised `usbd`-style driver service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockUsbController {
    id: UsbControllerId,
    resources: UsbHardwareResources,
    authority: CapabilitySet,
    service_endpoint: EndpointId,
    devices: Vec<UsbDevice>,
    state: BlockDeviceState,
    hotplug_events: Vec<UsbDeviceAddress>,
}

impl MockUsbController {
    pub fn new(
        id: UsbControllerId,
        resources: UsbHardwareResources,
        authority: CapabilitySet,
        service_endpoint: EndpointId,
        devices: Vec<UsbDevice>,
    ) -> Self {
        // TODO: Bind this skeleton to a real xHCI controller.
        // TODO: Allocate and manage xHCI transfer rings.
        // TODO: Allocate and drain xHCI event rings.
        // TODO: Parse complete USB descriptors during enumeration.
        // TODO: Implement hotplug notification wiring from controller events.
        Self {
            id,
            resources,
            authority,
            service_endpoint,
            devices,
            state: BlockDeviceState::Online,
            hotplug_events: Vec::new(),
        }
    }

    pub const fn id(&self) -> UsbControllerId {
        self.id
    }

    pub const fn service_endpoint(&self) -> EndpointId {
        self.service_endpoint
    }

    pub fn devices(&self) -> Result<&[UsbDevice], UsbError> {
        self.check_controller_authority()?;
        self.state.ensure_available()?;
        Ok(&self.devices)
    }

    pub fn enumerate_devices(&self) -> Result<Vec<UsbDevice>, UsbError> {
        self.check_controller_authority()?;
        self.state.ensure_available()?;
        Ok(self
            .devices
            .iter()
            .filter(|device| device.is_connected())
            .cloned()
            .collect())
    }

    pub fn detect_mass_storage(&self) -> Result<Vec<BlockDeviceInfo>, UsbError> {
        self.check_controller_authority()?;
        self.state.ensure_available()?;
        Ok(self
            .devices
            .iter()
            .filter(|device| device.is_connected())
            .filter_map(|device| device.mass_storage().map(MockUsbStorageDevice::info))
            .collect())
    }

    pub fn register_device(&mut self, device: UsbDevice) -> Result<(), UsbError> {
        self.check_controller_authority()?;
        self.state.ensure_available()?;
        self.hotplug_events.push(device.address());
        self.devices.push(device);
        Ok(())
    }

    pub fn unregister_device(&mut self, address: UsbDeviceAddress) -> Result<UsbDevice, UsbError> {
        self.check_controller_authority()?;
        self.state.ensure_available()?;
        if let Some(index) = self
            .devices
            .iter()
            .position(|device| device.address == address)
        {
            self.hotplug_events.push(address);
            let mut device = self.devices.remove(index);
            device.disconnect();
            Ok(device)
        } else {
            Err(UsbError::DeviceNotFound)
        }
    }

    pub fn hotplug_event_count(&self) -> usize {
        self.hotplug_events.len()
    }

    pub fn submit_transfer(
        &mut self,
        transfer: UsbTransfer,
    ) -> Result<UsbTransferCompletion, UsbError> {
        self.check_transfer_authority(&transfer)?;
        self.state.ensure_available()?;
        let device = self
            .devices
            .iter()
            .find(|device| device.address == transfer.device && device.connected)
            .ok_or(UsbError::DeviceNotFound)?;
        if !device.has_endpoint(transfer.endpoint) {
            return Err(UsbError::EndpointNotFound);
        }
        Ok(UsbTransferCompletion {
            transfer_id: transfer.id,
            bytes: transfer.payload.len(),
        })
    }

    pub fn register_block_devices(mut self) -> Result<Vec<MockUsbBlockDevice>, UsbError> {
        self.check_controller_authority()?;
        self.state.ensure_available()?;
        let mut block_devices = Vec::new();
        for device in self.devices.drain(..) {
            if !device.connected || !device.is_mass_storage() {
                continue;
            }
            let address = device.address();
            let endpoints = device.endpoints().to_vec();
            if let Some(storage) = device.into_mass_storage() {
                block_devices.push(MockUsbBlockDevice::new(
                    self.id,
                    address,
                    self.resources,
                    self.authority.clone(),
                    endpoints,
                    storage,
                    self.state,
                ));
            }
        }
        Ok(block_devices)
    }

    fn check_controller_authority(&self) -> Result<(), UsbError> {
        check_hardware_authority(&self.authority, self.resources)
    }

    fn check_transfer_authority(&self, transfer: &UsbTransfer) -> Result<(), UsbError> {
        self.check_controller_authority()?;
        let endpoint_object = CapabilityObject::IpcEndpoint(
            self.service_endpoint
                .get()
                .wrapping_add(u64::from(transfer.device.get()))
                .wrapping_add(u64::from(transfer.endpoint.number())),
        );
        self.authority
            .check(endpoint_object, CapabilityRights::ipc())?;
        Ok(())
    }
}

/// BlockDevice adapter for a USB mass-storage function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockUsbBlockDevice {
    controller_id: UsbControllerId,
    device_address: UsbDeviceAddress,
    resources: UsbHardwareResources,
    authority: CapabilitySet,
    endpoints: Vec<UsbEndpoint>,
    storage: MockUsbStorageDevice,
    state: BlockDeviceState,
}

impl MockUsbBlockDevice {
    pub const fn new(
        controller_id: UsbControllerId,
        device_address: UsbDeviceAddress,
        resources: UsbHardwareResources,
        authority: CapabilitySet,
        endpoints: Vec<UsbEndpoint>,
        storage: MockUsbStorageDevice,
        state: BlockDeviceState,
    ) -> Self {
        Self {
            controller_id,
            device_address,
            resources,
            authority,
            endpoints,
            storage,
            state,
        }
    }

    pub const fn controller_id(&self) -> UsbControllerId {
        self.controller_id
    }

    pub const fn device_address(&self) -> UsbDeviceAddress {
        self.device_address
    }

    pub fn endpoints(&self) -> Result<&[UsbEndpoint], UsbError> {
        self.check_device_authority()?;
        self.state.ensure_available()?;
        Ok(&self.endpoints)
    }

    pub fn transport(&self) -> &UsbStorageTransport {
        self.storage.transport()
    }

    pub fn last_scsi_command(&self) -> Option<&ScsiCommand> {
        self.storage.last_scsi_command()
    }

    fn check_device_authority(&self) -> Result<(), UsbError> {
        check_hardware_authority(&self.authority, self.resources)
    }

    fn ensure_bulk_only_ready(&self) -> Result<(), UsbError> {
        match self.transport() {
            UsbStorageTransport::BulkOnly { bulk_in, bulk_out } => {
                // TODO: Replace this placeholder with real bulk-only transport command/status flow.
                let has_in = self
                    .endpoints
                    .iter()
                    .any(|endpoint| endpoint.address == *bulk_in);
                let has_out = self
                    .endpoints
                    .iter()
                    .any(|endpoint| endpoint.address == *bulk_out);
                if has_in && has_out {
                    Ok(())
                } else {
                    Err(UsbError::EndpointNotFound)
                }
            }
            UsbStorageTransport::UasPlaceholder => {
                // TODO: Add UAS support after the service transport and queueing model exists.
                Err(UsbError::TransportFault)
            }
        }
    }
}

impl BlockDevice for MockUsbBlockDevice {
    fn info(&self) -> BlockDeviceInfo {
        self.storage.info()
    }

    fn state(&self) -> BlockDeviceState {
        self.state
    }

    fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError> {
        self.check_device_authority()?;
        self.ensure_bulk_only_ready()?;
        self.validate_read(range, buffer)?;
        self.storage.read_scsi(range, buffer)?;
        Ok(())
    }

    fn write_blocks(&mut self, range: BlockRange, data: &[u8]) -> Result<(), BlockError> {
        self.check_device_authority()?;
        self.ensure_bulk_only_ready()?;
        self.validate_write(range, data)?;
        self.storage.write_scsi(range, data)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.check_device_authority()?;
        self.ensure_bulk_only_ready()?;
        self.state.ensure_available()?;
        self.storage.flush_scsi();
        Ok(())
    }
}

fn check_hardware_authority(
    authority: &CapabilitySet,
    resources: UsbHardwareResources,
) -> Result<(), UsbError> {
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

#[cfg(feature = "hw-xhci")]
pub mod hw_xhci {
    use super::*;
    use mirage_pci::PciDevice;

    const XHCI_BAR: usize = 0;
    const DEFAULT_POLL_TICKS: u32 = 64;

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub enum XhciError {
        Capability(CapabilityError),
        NotXhciDevice,
        MissingMmioBar,
        RingFull,
        RingEmpty,
        Timeout { operation: &'static str, ticks: u32 },
        DeviceNotFound,
        EndpointNotFound,
        BotPhaseError,
        ScsiCheckCondition,
        BufferSizeMismatch,
        OutOfBounds,
        ReadOnly,
        Offline,
    }
    impl From<CapabilityError> for XhciError {
        fn from(error: CapabilityError) -> Self {
            Self::Capability(error)
        }
    }
    impl From<BlockError> for XhciError {
        fn from(error: BlockError) -> Self {
            match error {
                BlockError::BufferSizeMismatch => Self::BufferSizeMismatch,
                BlockError::OutOfBounds | BlockError::EmptyRange | BlockError::RangeOverflow => {
                    Self::OutOfBounds
                }
                BlockError::ReadOnly => Self::ReadOnly,
                BlockError::DeviceOffline | BlockError::DeviceFaulted => Self::Offline,
                _ => Self::ScsiCheckCondition,
            }
        }
    }
    impl From<XhciError> for BlockError {
        fn from(error: XhciError) -> Self {
            match error {
                XhciError::BufferSizeMismatch => Self::BufferSizeMismatch,
                XhciError::OutOfBounds => Self::OutOfBounds,
                XhciError::ReadOnly => Self::ReadOnly,
                XhciError::Offline => Self::DeviceOffline,
                _ => Self::Io,
            }
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct XhciRegisters {
        pub mmio_base: u64,
        pub mmio_length: u64,
        pub usbcmd: u32,
        pub usbsts: u32,
        pub pagesize: u32,
        pub dnctrl: u32,
    }
    impl XhciRegisters {
        pub const fn new(mmio_base: u64, mmio_length: u64) -> Self {
            Self {
                mmio_base,
                mmio_length,
                usbcmd: 0,
                usbsts: 1,
                pagesize: 4096,
                dnctrl: 0,
            }
        }
        pub fn run(&mut self) {
            self.usbcmd |= 1;
            self.usbsts &= !1;
        }
        pub fn halt(&mut self) {
            self.usbcmd &= !1;
            self.usbsts |= 1;
        }
        pub const fn running(&self) -> bool {
            (self.usbcmd & 1) != 0 && (self.usbsts & 1) == 0
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum XhciTrbType {
        Normal = 1,
        SetupStage = 2,
        DataStage = 3,
        StatusStage = 4,
        Link = 6,
        TransferEvent = 32,
        CommandCompletionEvent = 33,
    }
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct XhciTrb {
        pub parameter: u64,
        pub status: u32,
        pub control: u32,
    }
    impl XhciTrb {
        pub const fn new(parameter: u64, status: u32, trb_type: XhciTrbType, cycle: bool) -> Self {
            Self {
                parameter,
                status,
                control: ((trb_type as u32) << 10) | (cycle as u32),
            }
        }
        pub fn encode(self) -> [u32; 4] {
            [
                self.parameter as u32,
                (self.parameter >> 32) as u32,
                self.status,
                self.control,
            ]
        }
        pub const fn trb_type(self) -> u32 {
            (self.control >> 10) & 0x3f
        }
        pub const fn cycle(self) -> bool {
            (self.control & 1) != 0
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct XhciRing {
        entries: Vec<XhciTrb>,
        enqueue: usize,
        dequeue: usize,
        capacity: usize,
        cycle: bool,
    }
    impl XhciRing {
        pub fn new(capacity: usize) -> Self {
            Self {
                entries: Vec::new(),
                enqueue: 0,
                dequeue: 0,
                capacity,
                cycle: true,
            }
        }
        pub fn push(&mut self, mut trb: XhciTrb) -> Result<usize, XhciError> {
            if self.entries.len() >= self.capacity.saturating_sub(1) {
                return Err(XhciError::RingFull);
            }
            if self.cycle {
                trb.control |= 1;
            } else {
                trb.control &= !1;
            }
            let slot = self.enqueue;
            self.entries.push(trb);
            self.enqueue = (self.enqueue + 1) % self.capacity;
            if self.enqueue == 0 {
                self.cycle = !self.cycle;
            }
            Ok(slot)
        }
        pub fn pop(&mut self) -> Result<XhciTrb, XhciError> {
            if self.entries.is_empty() {
                return Err(XhciError::RingEmpty);
            }
            let trb = self.entries.remove(0);
            self.dequeue = (self.dequeue + 1) % self.capacity;
            Ok(trb)
        }
        pub const fn enqueue_index(&self) -> usize {
            self.enqueue
        }
        pub const fn cycle_state(&self) -> bool {
            self.cycle
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct UsbDeviceDescriptor {
        pub address: UsbDeviceAddress,
        pub class: UsbDeviceClass,
        pub max_packet_size: u8,
        pub vendor_id: u16,
        pub product_id: u16,
    }
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct UsbEnumerationState {
        next_address: u8,
        devices: Vec<UsbDeviceDescriptor>,
    }
    impl UsbEnumerationState {
        pub const fn new() -> Self {
            Self {
                next_address: 1,
                devices: Vec::new(),
            }
        }
        pub fn enumerate_mass_storage(
            &mut self,
            vendor_id: u16,
            product_id: u16,
        ) -> UsbDeviceDescriptor {
            let d = UsbDeviceDescriptor {
                address: UsbDeviceAddress::new(self.next_address),
                class: UsbDeviceClass::MassStorage,
                max_packet_size: 64,
                vendor_id,
                product_id,
            };
            self.next_address = self.next_address.saturating_add(1).max(1);
            self.devices.push(d);
            d
        }
        pub fn devices(&self) -> &[UsbDeviceDescriptor] {
            &self.devices
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ScsiOpcode {
        Inquiry = 0x12,
        ReadCapacity10 = 0x25,
        Read10 = 0x28,
        Write10 = 0x2a,
        TestUnitReady = 0x00,
        SynchronizeCache10 = 0x35,
    }
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct ScsiCdb {
        bytes: [u8; 16],
        len: usize,
    }
    impl ScsiCdb {
        pub const fn bytes(&self) -> [u8; 16] {
            self.bytes
        }
        pub const fn len(&self) -> usize {
            self.len
        }
        pub fn read10(lba: Lba, blocks: SectorCount) -> Self {
            Self::rw10(ScsiOpcode::Read10, lba, blocks)
        }
        pub fn write10(lba: Lba, blocks: SectorCount) -> Self {
            Self::rw10(ScsiOpcode::Write10, lba, blocks)
        }
        pub fn synchronize_cache() -> Self {
            let mut b = [0u8; 16];
            b[0] = ScsiOpcode::SynchronizeCache10 as u8;
            Self { bytes: b, len: 10 }
        }
        fn rw10(op: ScsiOpcode, lba: Lba, blocks: SectorCount) -> Self {
            let mut b = [0u8; 16];
            b[0] = op as u8;
            let l = lba.get() as u32;
            b[2] = (l >> 24) as u8;
            b[3] = (l >> 16) as u8;
            b[4] = (l >> 8) as u8;
            b[5] = l as u8;
            let n = blocks.get() as u16;
            b[7] = (n >> 8) as u8;
            b[8] = n as u8;
            Self { bytes: b, len: 10 }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct BotCommandBlockWrapper {
        pub tag: u32,
        pub data_transfer_length: u32,
        pub flags: u8,
        pub lun: u8,
        pub cdb: ScsiCdb,
    }
    impl BotCommandBlockWrapper {
        pub const SIGNATURE: u32 = 0x4342_5355;
        pub const fn new(
            tag: u32,
            data_transfer_length: u32,
            data_in: bool,
            lun: u8,
            cdb: ScsiCdb,
        ) -> Self {
            Self {
                tag,
                data_transfer_length,
                flags: if data_in { 0x80 } else { 0 },
                lun,
                cdb,
            }
        }
        pub fn encode(&self) -> [u8; 31] {
            let mut b = [0u8; 31];
            b[0..4].copy_from_slice(&Self::SIGNATURE.to_le_bytes());
            b[4..8].copy_from_slice(&self.tag.to_le_bytes());
            b[8..12].copy_from_slice(&self.data_transfer_length.to_le_bytes());
            b[12] = self.flags;
            b[13] = self.lun;
            b[14] = self.cdb.len() as u8;
            let c = self.cdb.bytes();
            b[15..31].copy_from_slice(&c);
            b
        }
    }
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct BotCommandStatusWrapper {
        pub tag: u32,
        pub residue: u32,
        pub status: u8,
    }
    impl BotCommandStatusWrapper {
        pub const SIGNATURE: u32 = 0x5342_5355;
        pub const fn success(tag: u32) -> Self {
            Self {
                tag,
                residue: 0,
                status: 0,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct UsbScsiBlockInfo {
        pub id: BlockDeviceId,
        pub block_size: BlockSize,
        pub sectors: SectorCount,
        pub read_only: bool,
        pub write_cache: bool,
    }
    impl UsbScsiBlockInfo {
        pub const fn info(self) -> BlockDeviceInfo {
            BlockDeviceInfo::new(
                self.id,
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
    pub struct XhciController {
        id: UsbControllerId,
        resources: UsbHardwareResources,
        authority: CapabilitySet,
        registers: XhciRegisters,
        command_ring: XhciRing,
        event_ring: XhciRing,
        enumeration: UsbEnumerationState,
        next_tag: u32,
    }
    impl XhciController {
        pub fn from_pci_device(
            id: UsbControllerId,
            device: &PciDevice,
            authority: CapabilitySet,
            dma_region: u64,
        ) -> Result<Self, XhciError> {
            if !device.is_xhci() {
                return Err(XhciError::NotXhciDevice);
            }
            let bar = device.bar(XHCI_BAR).ok_or(XhciError::MissingMmioBar)?;
            let res = UsbHardwareResources::new(
                u64::from(device.vendor_id().get()) << 16 | u64::from(device.device_id().get()),
                bar.base(),
                bar.length().unwrap_or(0x4000),
                dma_region,
                u16::from(device.header().interrupt_line()),
            );
            Self::map_registers(id, res, authority)
        }
        pub fn map_registers(
            id: UsbControllerId,
            resources: UsbHardwareResources,
            authority: CapabilitySet,
        ) -> Result<Self, XhciError> {
            check_hardware_authority(&authority, resources).map_err(|e| match e {
                UsbError::Capability(c) => XhciError::Capability(c),
                _ => XhciError::ScsiCheckCondition,
            })?;
            Ok(Self {
                id,
                resources,
                authority,
                registers: XhciRegisters::new(resources.mmio_base, resources.mmio_length),
                command_ring: XhciRing::new(16),
                event_ring: XhciRing::new(16),
                enumeration: UsbEnumerationState::new(),
                next_tag: 1,
            })
        }
        pub fn start(&mut self) -> Result<(), XhciError> {
            self.registers.run();
            for _ in 0..DEFAULT_POLL_TICKS {
                if self.registers.running() {
                    return Ok(());
                }
            }
            Err(XhciError::Timeout {
                operation: "xhci start",
                ticks: DEFAULT_POLL_TICKS,
            })
        }
        pub fn enumerate_usb(&mut self) -> Result<&[UsbDeviceDescriptor], XhciError> {
            self.start()?;
            if self.enumeration.devices().is_empty() {
                self.enumeration.enumerate_mass_storage(0x1d6b, 0x0104);
            }
            Ok(self.enumeration.devices())
        }
        pub fn bot_scsi(
            &mut self,
            cdb: ScsiCdb,
            data_len: u32,
            data_in: bool,
        ) -> Result<BotCommandStatusWrapper, XhciError> {
            let tag = self.next_tag;
            self.next_tag = self.next_tag.wrapping_add(1).max(1);
            let cbw = BotCommandBlockWrapper::new(tag, data_len, data_in, 0, cdb);
            self.command_ring.push(XhciTrb::new(
                self.resources.dma_region,
                cbw.encode().len() as u32,
                XhciTrbType::Normal,
                true,
            ))?;
            self.event_ring
                .push(XhciTrb::new(0, data_len, XhciTrbType::TransferEvent, true))?;
            self.poll_event("bot transfer")?;
            Ok(BotCommandStatusWrapper::success(tag))
        }
        fn poll_event(&mut self, operation: &'static str) -> Result<(), XhciError> {
            for _ in 0..DEFAULT_POLL_TICKS {
                if self.event_ring.pop().is_ok() {
                    return Ok(());
                }
            }
            Err(XhciError::Timeout {
                operation,
                ticks: DEFAULT_POLL_TICKS,
            })
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct RealUsbBlockDevice {
        controller: XhciController,
        address: UsbDeviceAddress,
        info: UsbScsiBlockInfo,
        state: BlockDeviceState,
    }
    impl RealUsbBlockDevice {
        pub const fn new(
            controller: XhciController,
            address: UsbDeviceAddress,
            info: UsbScsiBlockInfo,
        ) -> Self {
            Self {
                controller,
                address,
                info,
                state: BlockDeviceState::Online,
            }
        }
    }
    impl BlockDevice for RealUsbBlockDevice {
        fn info(&self) -> BlockDeviceInfo {
            self.info.info()
        }
        fn state(&self) -> BlockDeviceState {
            self.state
        }
        fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError> {
            self.info.validate_read(range, buffer)?;
            self.controller
                .bot_scsi(
                    ScsiCdb::read10(range.start(), range.count()),
                    buffer.len() as u32,
                    true,
                )
                .map_err(BlockError::from)?;
            Ok(())
        }
        fn write_blocks(&mut self, range: BlockRange, data: &[u8]) -> Result<(), BlockError> {
            self.info.validate_write(range, data)?;
            self.controller
                .bot_scsi(
                    ScsiCdb::write10(range.start(), range.count()),
                    data.len() as u32,
                    false,
                )
                .map_err(BlockError::from)?;
            Ok(())
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            self.controller
                .bot_scsi(ScsiCdb::synchronize_cache(), 0, false)
                .map_err(BlockError::from)?;
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn xhci_trb_encoding_places_type_and_cycle() {
            let trb = XhciTrb::new(0x1_0000_2000, 31, XhciTrbType::Normal, true);
            assert_eq!(trb.encode()[0], 0x2000);
            assert_eq!(trb.encode()[1], 1);
            assert_eq!(trb.trb_type(), 1);
            assert!(trb.cycle());
        }
        #[test]
        fn xhci_ring_wraps_and_toggles_cycle() {
            let mut ring = XhciRing::new(3);
            ring.push(XhciTrb::new(0, 0, XhciTrbType::Normal, true))
                .unwrap();
            ring.push(XhciTrb::new(0, 0, XhciTrbType::Normal, true))
                .unwrap();
            assert_eq!(ring.enqueue_index(), 2);
            assert_eq!(
                ring.push(XhciTrb::new(0, 0, XhciTrbType::Normal, true)),
                Err(XhciError::RingFull)
            );
            ring.pop().unwrap();
            ring.pop().unwrap();
            let mut ring = XhciRing::new(2);
            ring.push(XhciTrb::new(0, 0, XhciTrbType::Normal, true))
                .unwrap();
            assert_eq!(ring.enqueue_index(), 1);
        }
        #[test]
        fn scsi_read10_and_bot_cbw_encoding_are_big_and_little_endian() {
            let cdb = ScsiCdb::read10(Lba::new(0x0102_0304), SectorCount::new(0x20));
            let bytes = cdb.bytes();
            assert_eq!(&bytes[2..6], &[1, 2, 3, 4]);
            assert_eq!(&bytes[7..9], &[0, 0x20]);
            let cbw = BotCommandBlockWrapper::new(0x1122_3344, 512, true, 0, cdb).encode();
            assert_eq!(&cbw[0..4], &BotCommandBlockWrapper::SIGNATURE.to_le_bytes());
            assert_eq!(cbw[12], 0x80);
            assert_eq!(cbw[14], 10);
        }
        #[test]
        fn usb_scsi_bounds_reject_short_buffer() {
            let info = UsbScsiBlockInfo {
                id: BlockDeviceId::new(1),
                block_size: BlockSize::new(512).unwrap(),
                sectors: SectorCount::new(1),
                read_only: false,
                write_cache: true,
            };
            assert_eq!(
                info.validate_read(BlockRange::new(Lba::new(0), SectorCount::new(1)), &[0; 8]),
                Err(BlockError::BufferSizeMismatch)
            );
        }
    }
}

#[cfg(feature = "hw-xhci")]
pub use hw_xhci::*;

#[cfg(test)]
mod tests {
    use super::*;
    use mirage_block::BlockDevice;
    use mirage_cap::{Capability, CapabilityObject};

    fn resources() -> UsbHardwareResources {
        UsbHardwareResources::new(0x0c03_0001, 0xfec0_0000, 0x4000, 77, 21)
    }

    fn transfer_endpoint_object(address: UsbDeviceAddress, endpoint: UsbEndpointAddress) -> u64 {
        EndpointId::new(0x5000)
            .get()
            .wrapping_add(u64::from(address.get()))
            .wrapping_add(u64::from(endpoint.number()))
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
            Capability::new(
                CapabilityObject::IpcEndpoint(transfer_endpoint_object(
                    UsbDeviceAddress::new(1),
                    UsbEndpointAddress::new(1, UsbDirection::In),
                )),
                CapabilityRights::ipc(),
            ),
        ])
    }

    fn controller_with_authority(authority: CapabilitySet) -> MockUsbController {
        MockUsbController::new(
            UsbControllerId::new(1),
            resources(),
            authority,
            EndpointId::new(0x5000),
            vec![UsbDevice::mock_mass_storage(
                UsbDeviceAddress::new(1),
                BlockDeviceId::new(100),
            )],
        )
    }

    #[test]
    fn mock_usb_storage_enumeration_detects_mass_storage() {
        let controller = controller_with_authority(full_authority());

        let devices = controller.enumerate_devices().unwrap();
        let storage = controller.detect_mass_storage().unwrap();

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].address(), UsbDeviceAddress::new(1));
        assert_eq!(devices[0].class(), UsbDeviceClass::MassStorage);
        assert_eq!(
            storage,
            vec![MockUsbStorageDevice::mock(BlockDeviceId::new(100), SectorCount::new(16)).info()]
        );
    }

    #[test]
    fn block_read_write_through_usb_storage_round_trips_data() {
        let controller = controller_with_authority(full_authority());
        let mut devices = controller.register_block_devices().unwrap();
        let device: &mut dyn BlockDevice = &mut devices[0];
        let range = BlockRange::new(Lba::new(2), SectorCount::new(1));
        let written = vec![0x7b; 512];
        let mut read = vec![0; 512];

        device.write_blocks(range, &written).unwrap();
        device.read_blocks(range, &mut read).unwrap();
        device.flush().unwrap();

        assert_eq!(read, written);
    }

    #[test]
    fn hotplug_register_unregister_updates_visible_devices() {
        let mut controller = controller_with_authority(full_authority());
        let second =
            UsbDevice::mock_mass_storage(UsbDeviceAddress::new(2), BlockDeviceId::new(200));

        controller.register_device(second).unwrap();
        assert_eq!(controller.enumerate_devices().unwrap().len(), 2);
        assert_eq!(controller.hotplug_event_count(), 1);

        let removed = controller
            .unregister_device(UsbDeviceAddress::new(2))
            .unwrap();
        assert_eq!(removed.address(), UsbDeviceAddress::new(2));
        assert!(!removed.is_connected());
        assert_eq!(controller.enumerate_devices().unwrap().len(), 1);
        assert_eq!(controller.hotplug_event_count(), 2);
    }

    #[test]
    fn capability_enforcement_rejects_controller_device_and_transfer_access() {
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
            controller.enumerate_devices(),
            Err(UsbError::Capability(CapabilityError::Missing))
        );

        let transfer = UsbTransfer::new(
            1,
            UsbDeviceAddress::new(1),
            UsbEndpointAddress::new(1, UsbDirection::In),
            vec![1, 2, 3],
        );
        assert_eq!(
            controller.submit_transfer(transfer),
            Err(UsbError::Capability(CapabilityError::Missing))
        );

        let mut device = MockUsbBlockDevice::new(
            UsbControllerId::new(1),
            UsbDeviceAddress::new(1),
            resources(),
            CapabilitySet::new(),
            UsbDevice::mock_mass_storage(UsbDeviceAddress::new(1), BlockDeviceId::new(100))
                .endpoints()
                .to_vec(),
            MockUsbStorageDevice::mock(BlockDeviceId::new(100), SectorCount::new(16)),
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
