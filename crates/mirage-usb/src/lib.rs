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
pub struct UsbMassStorageDevice {
    info: BlockDeviceInfo,
    transport: UsbStorageTransport,
    storage: Vec<u8>,
    last_scsi_command: Option<ScsiCommand>,
}

impl UsbMassStorageDevice {
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
    mass_storage: Option<UsbMassStorageDevice>,
    connected: bool,
}

impl UsbDevice {
    pub fn new(
        address: UsbDeviceAddress,
        class: UsbDeviceClass,
        endpoints: Vec<UsbEndpoint>,
        mass_storage: Option<UsbMassStorageDevice>,
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
            Some(UsbMassStorageDevice::mock(block_id, SectorCount::new(16))),
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

    pub fn mass_storage(&self) -> Option<&UsbMassStorageDevice> {
        self.mass_storage.as_ref()
    }

    fn into_mass_storage(self) -> Option<UsbMassStorageDevice> {
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
pub struct UsbController {
    id: UsbControllerId,
    resources: UsbHardwareResources,
    authority: CapabilitySet,
    service_endpoint: EndpointId,
    devices: Vec<UsbDevice>,
    state: BlockDeviceState,
    hotplug_events: Vec<UsbDeviceAddress>,
}

impl UsbController {
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
            .filter_map(|device| device.mass_storage().map(UsbMassStorageDevice::info))
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

    pub fn register_block_devices(mut self) -> Result<Vec<UsbBlockDevice>, UsbError> {
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
                block_devices.push(UsbBlockDevice::new(
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
pub struct UsbBlockDevice {
    controller_id: UsbControllerId,
    device_address: UsbDeviceAddress,
    resources: UsbHardwareResources,
    authority: CapabilitySet,
    endpoints: Vec<UsbEndpoint>,
    storage: UsbMassStorageDevice,
    state: BlockDeviceState,
}

impl UsbBlockDevice {
    pub const fn new(
        controller_id: UsbControllerId,
        device_address: UsbDeviceAddress,
        resources: UsbHardwareResources,
        authority: CapabilitySet,
        endpoints: Vec<UsbEndpoint>,
        storage: UsbMassStorageDevice,
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

impl BlockDevice for UsbBlockDevice {
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

    fn controller_with_authority(authority: CapabilitySet) -> UsbController {
        UsbController::new(
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
            vec![UsbMassStorageDevice::mock(BlockDeviceId::new(100), SectorCount::new(16)).info()]
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

        let mut device = UsbBlockDevice::new(
            UsbControllerId::new(1),
            UsbDeviceAddress::new(1),
            resources(),
            CapabilitySet::new(),
            UsbDevice::mock_mass_storage(UsbDeviceAddress::new(1), BlockDeviceId::new(100))
                .endpoints()
                .to_vec(),
            UsbMassStorageDevice::mock(BlockDeviceId::new(100), SectorCount::new(16)),
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
