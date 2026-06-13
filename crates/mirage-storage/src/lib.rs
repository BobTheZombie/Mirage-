#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec;
use alloc::vec::Vec;

use mirage_block::{
    BlockDevice, BlockDeviceId, BlockDeviceInfo, BlockError, BlockRange, Lba, SectorCount,
};

/// Errors returned by the supervisor-facing storage service layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageError {
    DeviceAlreadyRegistered,
    DeviceNotFound,
    AccessDenied,
    CapabilityRevoked,
    Block(BlockError),
}

impl From<BlockError> for StorageError {
    fn from(error: BlockError) -> Self {
        Self::Block(error)
    }
}

/// Access modes mediated by a [`StorageCapability`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageAccess {
    Read,
    Write,
    Flush,
}

/// Capability token granting scoped authority over one registered block device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageCapability {
    device_id: BlockDeviceId,
    can_read: bool,
    can_write: bool,
    can_flush: bool,
    revoked: bool,
}

impl StorageCapability {
    pub const fn new(
        device_id: BlockDeviceId,
        can_read: bool,
        can_write: bool,
        can_flush: bool,
    ) -> Self {
        Self {
            device_id,
            can_read,
            can_write,
            can_flush,
            revoked: false,
        }
    }

    pub const fn read_only(device_id: BlockDeviceId) -> Self {
        Self::new(device_id, true, false, false)
    }

    pub const fn read_write(device_id: BlockDeviceId) -> Self {
        Self::new(device_id, true, true, true)
    }

    pub const fn device_id(&self) -> BlockDeviceId {
        self.device_id
    }

    pub const fn is_revoked(&self) -> bool {
        self.revoked
    }

    pub fn revoke(&mut self) {
        self.revoked = true;
    }

    pub fn permits(
        &self,
        device_id: BlockDeviceId,
        access: StorageAccess,
    ) -> Result<(), StorageError> {
        if self.revoked {
            return Err(StorageError::CapabilityRevoked);
        }

        if self.device_id != device_id {
            return Err(StorageError::AccessDenied);
        }

        let allowed = match access {
            StorageAccess::Read => self.can_read,
            StorageAccess::Write => self.can_write,
            StorageAccess::Flush => self.can_flush,
        };

        if allowed {
            Ok(())
        } else {
            Err(StorageError::AccessDenied)
        }
    }
}

/// Stable QFS-facing view of a registered block device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageDeviceHandle {
    info: BlockDeviceInfo,
    partition_table: PartitionTable,
}

impl StorageDeviceHandle {
    pub const fn new(info: BlockDeviceInfo, partition_table: PartitionTable) -> Self {
        Self {
            info,
            partition_table,
        }
    }

    pub const fn id(&self) -> BlockDeviceId {
        self.info.id
    }

    pub const fn info(&self) -> BlockDeviceInfo {
        self.info
    }

    pub const fn partition_table(&self) -> &PartitionTable {
        &self.partition_table
    }
}

/// Storage hotplug/remove notifications emitted by [`StorageService`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageEvent {
    DeviceHotplugged(StorageDeviceHandle),
    DeviceRemoved(BlockDeviceId),
}

/// Minimal partition metadata placeholder for boot/QFS discovery prototypes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartitionInfo {
    pub index: u8,
    pub first_lba: Lba,
    pub sectors: SectorCount,
    pub type_code: u8,
    pub bootable: bool,
}

impl PartitionInfo {
    pub const fn new(
        index: u8,
        first_lba: Lba,
        sectors: SectorCount,
        type_code: u8,
        bootable: bool,
    ) -> Self {
        Self {
            index,
            first_lba,
            sectors,
            type_code,
            bootable,
        }
    }
}

/// Partition table probe result.
///
/// This is intentionally a lightweight placeholder: Mirage currently detects a few MBR/GPT
/// signatures for service wiring tests, but it does not claim complete parser support here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PartitionTable {
    Unknown,
    MbrPlaceholder { partitions: Vec<PartitionInfo> },
    GptPlaceholder { partitions: Vec<PartitionInfo> },
}

impl PartitionTable {
    pub fn partitions(&self) -> &[PartitionInfo] {
        match self {
            Self::Unknown => &[],
            Self::MbrPlaceholder { partitions } | Self::GptPlaceholder { partitions } => partitions,
        }
    }
}

struct RegisteredStorageDevice {
    device: Box<dyn BlockDevice>,
    handle: StorageDeviceHandle,
}

/// Registry of supervisor-owned block devices exposed to storage clients by handle.
#[derive(Default)]
pub struct StorageDeviceRegistry {
    devices: BTreeMap<BlockDeviceId, RegisteredStorageDevice>,
}

impl StorageDeviceRegistry {
    pub const fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
        }
    }

    pub fn register(
        &mut self,
        mut device: Box<dyn BlockDevice>,
    ) -> Result<StorageDeviceHandle, StorageError> {
        let info = device.info();
        if self.devices.contains_key(&info.id) {
            return Err(StorageError::DeviceAlreadyRegistered);
        }

        let partition_table = scan_partition_table(device.as_mut());
        let handle = StorageDeviceHandle::new(info, partition_table);
        self.devices.insert(
            info.id,
            RegisteredStorageDevice {
                device,
                handle: handle.clone(),
            },
        );
        Ok(handle)
    }

    pub fn unregister(
        &mut self,
        device_id: BlockDeviceId,
    ) -> Result<StorageDeviceHandle, StorageError> {
        self.devices
            .remove(&device_id)
            .map(|registered| registered.handle)
            .ok_or(StorageError::DeviceNotFound)
    }

    pub fn handle(&self, device_id: BlockDeviceId) -> Option<&StorageDeviceHandle> {
        self.devices
            .get(&device_id)
            .map(|registered| &registered.handle)
    }

    pub fn handles(&self) -> Vec<StorageDeviceHandle> {
        self.devices
            .values()
            .map(|registered| registered.handle.clone())
            .collect()
    }

    pub fn contains(&self, device_id: BlockDeviceId) -> bool {
        self.devices.contains_key(&device_id)
    }

    pub fn len(&self) -> usize {
        self.devices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    fn device_mut(
        &mut self,
        device_id: BlockDeviceId,
    ) -> Result<&mut dyn BlockDevice, StorageError> {
        match self.devices.get_mut(&device_id) {
            Some(registered) => Ok(registered.device.as_mut()),
            None => Err(StorageError::DeviceNotFound),
        }
    }
}

/// Supervisor-mediated storage service for QFS and other Mirage services.
#[derive(Default)]
pub struct StorageService {
    registry: StorageDeviceRegistry,
    events: VecDeque<StorageEvent>,
}

impl StorageService {
    pub const fn new() -> Self {
        Self {
            registry: StorageDeviceRegistry::new(),
            events: VecDeque::new(),
        }
    }

    pub fn registry(&self) -> &StorageDeviceRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut StorageDeviceRegistry {
        &mut self.registry
    }

    pub fn register_device(
        &mut self,
        device: Box<dyn BlockDevice>,
    ) -> Result<StorageDeviceHandle, StorageError> {
        let handle = self.registry.register(device)?;
        self.events
            .push_back(StorageEvent::DeviceHotplugged(handle.clone()));
        Ok(handle)
    }

    pub fn unregister_device(
        &mut self,
        device_id: BlockDeviceId,
    ) -> Result<StorageDeviceHandle, StorageError> {
        let handle = self.registry.unregister(device_id)?;
        self.events
            .push_back(StorageEvent::DeviceRemoved(device_id));
        Ok(handle)
    }

    pub fn next_event(&mut self) -> Option<StorageEvent> {
        self.events.pop_front()
    }

    pub fn pending_events(&self) -> usize {
        self.events.len()
    }

    pub fn grant_read_only(
        &self,
        device_id: BlockDeviceId,
    ) -> Result<StorageCapability, StorageError> {
        self.ensure_registered(device_id)?;
        Ok(StorageCapability::read_only(device_id))
    }

    pub fn grant_read_write(
        &self,
        device_id: BlockDeviceId,
    ) -> Result<StorageCapability, StorageError> {
        self.ensure_registered(device_id)?;
        Ok(StorageCapability::read_write(device_id))
    }

    pub fn read_blocks(
        &mut self,
        capability: &StorageCapability,
        handle: &StorageDeviceHandle,
        range: BlockRange,
        buffer: &mut [u8],
    ) -> Result<(), StorageError> {
        capability.permits(handle.id(), StorageAccess::Read)?;
        self.registry
            .device_mut(handle.id())?
            .read_blocks(range, buffer)
            .map_err(StorageError::from)
    }

    pub fn read_vec(
        &mut self,
        capability: &StorageCapability,
        handle: &StorageDeviceHandle,
        range: BlockRange,
    ) -> Result<Vec<u8>, StorageError> {
        capability.permits(handle.id(), StorageAccess::Read)?;
        let len = handle.info().expected_buffer_len(range)?;
        let mut buffer = vec![0; len];
        self.registry
            .device_mut(handle.id())?
            .read_blocks(range, &mut buffer)?;
        Ok(buffer)
    }

    pub fn write_blocks(
        &mut self,
        capability: &StorageCapability,
        handle: &StorageDeviceHandle,
        range: BlockRange,
        data: &[u8],
    ) -> Result<(), StorageError> {
        capability.permits(handle.id(), StorageAccess::Write)?;
        self.registry
            .device_mut(handle.id())?
            .write_blocks(range, data)
            .map_err(StorageError::from)
    }

    pub fn flush(
        &mut self,
        capability: &StorageCapability,
        handle: &StorageDeviceHandle,
    ) -> Result<(), StorageError> {
        capability.permits(handle.id(), StorageAccess::Flush)?;
        self.registry
            .device_mut(handle.id())?
            .flush()
            .map_err(StorageError::from)
    }

    fn ensure_registered(&self, device_id: BlockDeviceId) -> Result<(), StorageError> {
        if self.registry.contains(device_id) {
            Ok(())
        } else {
            Err(StorageError::DeviceNotFound)
        }
    }
}

fn scan_partition_table(device: &mut dyn BlockDevice) -> PartitionTable {
    let info = device.info();
    if info.sectors.get() == 0 {
        return PartitionTable::Unknown;
    }

    let sector_len = info.block_size.bytes_usize();
    let mut sector = vec![0; sector_len];
    let first_sector = BlockRange::new(Lba::new(0), SectorCount::new(1));
    if device.read_blocks(first_sector, &mut sector).is_err() {
        return PartitionTable::Unknown;
    }

    if info.sectors.get() > 1 {
        let mut gpt_header = vec![0; sector_len];
        let gpt_sector = BlockRange::new(Lba::new(1), SectorCount::new(1));
        if device.read_blocks(gpt_sector, &mut gpt_header).is_ok()
            && gpt_header.get(0..8) == Some(b"EFI PART".as_slice())
        {
            return PartitionTable::GptPlaceholder {
                partitions: Vec::new(),
            };
        }
    }

    if sector_len >= 512 && sector.get(510) == Some(&0x55) && sector.get(511) == Some(&0xAA) {
        let mut partitions = Vec::new();
        for index in 0..4 {
            let offset = 446 + index * 16;
            let entry = &sector[offset..offset + 16];
            let type_code = entry[4];
            let first_lba = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);
            let sectors = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]);
            if type_code != 0 && sectors != 0 {
                partitions.push(PartitionInfo::new(
                    index as u8 + 1,
                    Lba::new(first_lba as u64),
                    SectorCount::new(sectors as u64),
                    type_code,
                    entry[0] == 0x80,
                ));
            }
        }

        return PartitionTable::MbrPlaceholder { partitions };
    }

    PartitionTable::Unknown
}

#[cfg(feature = "discovery")]
pub mod discovery {
    #[cfg(any(
        feature = "discovery-nvme",
        feature = "discovery-ahci",
        feature = "discovery-xhci"
    ))]
    use alloc::boxed::Box;
    use alloc::vec::Vec;

    use mirage_cap::CapabilitySet;
    use mirage_pci::PciBus;
    #[cfg(any(
        feature = "discovery-nvme",
        feature = "discovery-ahci",
        feature = "discovery-xhci"
    ))]
    use mirage_pci::PciDevice;

    use super::{StorageDeviceHandle, StorageDeviceRegistry, StorageError, StorageService};

    /// Feature-gated storage discovery error.
    ///
    /// The concrete driver error stays inside the owning driver crate; storage discovery only
    /// reports which backend phase failed so generic storage and QFS code remain decoupled from
    /// PCI/NVMe/AHCI/USB policy and hardware details.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub enum DiscoveryError {
        DriverInitFailed { backend: StorageBackendKind },
        Storage(StorageError),
    }

    impl From<StorageError> for DiscoveryError {
        fn from(error: StorageError) -> Self {
            Self::Storage(error)
        }
    }

    /// Storage backend classes recognized by the optional PCI discovery flow.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum StorageBackendKind {
        Nvme,
        Ahci,
        UsbStorage,
    }

    /// Controller/DMA numbering policy supplied by the supervisor.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct DiscoveryConfig {
        pub controller_id_base: u64,
        pub dma_region_base: u64,
        pub dma_region_stride: u64,
    }

    impl DiscoveryConfig {
        pub const fn new(
            controller_id_base: u64,
            dma_region_base: u64,
            dma_region_stride: u64,
        ) -> Self {
            Self {
                controller_id_base,
                dma_region_base,
                dma_region_stride,
            }
        }

        #[cfg(any(
            feature = "discovery-nvme",
            feature = "discovery-ahci",
            feature = "discovery-xhci"
        ))]
        const fn controller_id(self, ordinal: u64) -> u64 {
            self.controller_id_base.wrapping_add(ordinal)
        }

        #[cfg(any(
            feature = "discovery-nvme",
            feature = "discovery-ahci",
            feature = "discovery-xhci"
        ))]
        const fn dma_region(self, ordinal: u64) -> u64 {
            self.dma_region_base
                .wrapping_add(self.dma_region_stride.wrapping_mul(ordinal))
        }
    }

    impl Default for DiscoveryConfig {
        fn default() -> Self {
            Self::new(1, 0x1000_0000, 0x10_0000)
        }
    }

    /// Handles registered by one discovery pass, grouped by backend.
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub struct DiscoveryReport {
        pub nvme: Vec<StorageDeviceHandle>,
        pub ahci: Vec<StorageDeviceHandle>,
        pub usb_storage: Vec<StorageDeviceHandle>,
    }

    impl DiscoveryReport {
        pub const fn new() -> Self {
            Self {
                nvme: Vec::new(),
                ahci: Vec::new(),
                usb_storage: Vec::new(),
            }
        }

        pub fn len(&self) -> usize {
            self.nvme.len() + self.ahci.len() + self.usb_storage.len()
        }

        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }

        #[cfg(any(
            feature = "discovery-nvme",
            feature = "discovery-ahci",
            feature = "discovery-xhci"
        ))]
        fn push(&mut self, backend: StorageBackendKind, handle: StorageDeviceHandle) {
            match backend {
                StorageBackendKind::Nvme => self.nvme.push(handle),
                StorageBackendKind::Ahci => self.ahci.push(handle),
                StorageBackendKind::UsbStorage => self.usb_storage.push(handle),
            }
        }
    }

    /// Scan a PCI bus, initialize enabled storage backends, and register block devices.
    pub fn discover_into_registry(
        bus: &PciBus,
        authority: CapabilitySet,
        config: DiscoveryConfig,
        registry: &mut StorageDeviceRegistry,
    ) -> Result<DiscoveryReport, DiscoveryError> {
        #[cfg_attr(
            not(any(
                feature = "discovery-nvme",
                feature = "discovery-ahci",
                feature = "discovery-xhci"
            )),
            allow(unused_mut)
        )]
        let mut report = DiscoveryReport::new();
        #[cfg(not(any(
            feature = "discovery-nvme",
            feature = "discovery-ahci",
            feature = "discovery-xhci"
        )))]
        let _ = (bus, authority, config, registry);

        #[cfg(any(
            feature = "discovery-nvme",
            feature = "discovery-ahci",
            feature = "discovery-xhci"
        ))]
        let mut ordinal = 0;

        #[cfg(any(
            feature = "discovery-nvme",
            feature = "discovery-ahci",
            feature = "discovery-xhci"
        ))]
        for device in bus.devices() {
            #[cfg(feature = "discovery-nvme")]
            if device.is_nvme() {
                let handles = init_nvme(device, authority.clone(), config, ordinal, registry)?;
                for handle in handles {
                    report.push(StorageBackendKind::Nvme, handle);
                }
                ordinal += 1;
                continue;
            }

            #[cfg(feature = "discovery-ahci")]
            if device.is_ahci() {
                let handles = init_ahci(device, authority.clone(), config, ordinal, registry)?;
                for handle in handles {
                    report.push(StorageBackendKind::Ahci, handle);
                }
                ordinal += 1;
                continue;
            }

            #[cfg(feature = "discovery-xhci")]
            if device.is_xhci() {
                let handles = init_xhci(device, authority.clone(), config, ordinal, registry)?;
                for handle in handles {
                    report.push(StorageBackendKind::UsbStorage, handle);
                }
                ordinal += 1;
            }
        }

        Ok(report)
    }

    /// Scan a PCI bus and register discovered devices through [`StorageService`].
    pub fn discover_into_service(
        bus: &PciBus,
        authority: CapabilitySet,
        config: DiscoveryConfig,
        service: &mut StorageService,
    ) -> Result<DiscoveryReport, DiscoveryError> {
        #[cfg_attr(
            not(any(
                feature = "discovery-nvme",
                feature = "discovery-ahci",
                feature = "discovery-xhci"
            )),
            allow(unused_mut)
        )]
        let mut report = DiscoveryReport::new();
        #[cfg(not(any(
            feature = "discovery-nvme",
            feature = "discovery-ahci",
            feature = "discovery-xhci"
        )))]
        let _ = (bus, authority, config, service);

        #[cfg(any(
            feature = "discovery-nvme",
            feature = "discovery-ahci",
            feature = "discovery-xhci"
        ))]
        let mut ordinal = 0;

        #[cfg(any(
            feature = "discovery-nvme",
            feature = "discovery-ahci",
            feature = "discovery-xhci"
        ))]
        for device in bus.devices() {
            #[cfg(feature = "discovery-nvme")]
            if device.is_nvme() {
                let handles =
                    init_nvme_service(device, authority.clone(), config, ordinal, service)?;
                for handle in handles {
                    report.push(StorageBackendKind::Nvme, handle);
                }
                ordinal += 1;
                continue;
            }

            #[cfg(feature = "discovery-ahci")]
            if device.is_ahci() {
                let handles =
                    init_ahci_service(device, authority.clone(), config, ordinal, service)?;
                for handle in handles {
                    report.push(StorageBackendKind::Ahci, handle);
                }
                ordinal += 1;
                continue;
            }

            #[cfg(feature = "discovery-xhci")]
            if device.is_xhci() {
                let handles =
                    init_xhci_service(device, authority.clone(), config, ordinal, service)?;
                for handle in handles {
                    report.push(StorageBackendKind::UsbStorage, handle);
                }
                ordinal += 1;
            }
        }

        Ok(report)
    }

    #[cfg(feature = "discovery-nvme")]
    fn init_nvme(
        device: &PciDevice,
        authority: CapabilitySet,
        config: DiscoveryConfig,
        ordinal: u64,
        registry: &mut StorageDeviceRegistry,
    ) -> Result<Vec<StorageDeviceHandle>, DiscoveryError> {
        let block_devices = nvme_block_devices(device, authority, config, ordinal)?;
        let mut handles = Vec::new();
        for block_device in block_devices {
            handles.push(registry.register(block_device)?);
        }
        Ok(handles)
    }

    #[cfg(feature = "discovery-nvme")]
    fn init_nvme_service(
        device: &PciDevice,
        authority: CapabilitySet,
        config: DiscoveryConfig,
        ordinal: u64,
        service: &mut StorageService,
    ) -> Result<Vec<StorageDeviceHandle>, DiscoveryError> {
        let block_devices = nvme_block_devices(device, authority, config, ordinal)?;
        let mut handles = Vec::new();
        for block_device in block_devices {
            handles.push(service.register_device(block_device)?);
        }
        Ok(handles)
    }

    #[cfg(feature = "discovery-nvme")]
    fn nvme_block_devices(
        device: &PciDevice,
        authority: CapabilitySet,
        config: DiscoveryConfig,
        ordinal: u64,
    ) -> Result<Vec<Box<dyn mirage_block::BlockDevice>>, DiscoveryError> {
        use mirage_nvme::{NvmeController, NvmeControllerId, RealNvmeBlockDevice};

        let mut controller = NvmeController::from_pci_device(
            NvmeControllerId::new(config.controller_id(ordinal)),
            device,
            authority,
            config.dma_region(ordinal),
        )
        .map_err(|_| DiscoveryError::DriverInitFailed {
            backend: StorageBackendKind::Nvme,
        })?;
        controller
            .reset()
            .and_then(|_| controller.init_admin_queue(16))
            .and_then(|_| controller.identify_controller().map(|_| ()))
            .and_then(|_| controller.create_io_queue_pair(1, 16))
            .map_err(|_| DiscoveryError::DriverInitFailed {
                backend: StorageBackendKind::Nvme,
            })?;

        let namespaces = controller
            .identify_namespaces()
            .map_err(|_| DiscoveryError::DriverInitFailed {
                backend: StorageBackendKind::Nvme,
            })?
            .to_vec();
        let mut devices: Vec<Box<dyn mirage_block::BlockDevice>> = Vec::new();
        for namespace in namespaces {
            devices.push(Box::new(RealNvmeBlockDevice::new(
                controller.clone(),
                namespace,
            )));
        }
        Ok(devices)
    }

    #[cfg(feature = "discovery-ahci")]
    fn init_ahci(
        device: &PciDevice,
        authority: CapabilitySet,
        config: DiscoveryConfig,
        ordinal: u64,
        registry: &mut StorageDeviceRegistry,
    ) -> Result<Vec<StorageDeviceHandle>, DiscoveryError> {
        let block_devices = ahci_block_devices(device, authority, config, ordinal)?;
        let mut handles = Vec::new();
        for block_device in block_devices {
            handles.push(registry.register(block_device)?);
        }
        Ok(handles)
    }

    #[cfg(feature = "discovery-ahci")]
    fn init_ahci_service(
        device: &PciDevice,
        authority: CapabilitySet,
        config: DiscoveryConfig,
        ordinal: u64,
        service: &mut StorageService,
    ) -> Result<Vec<StorageDeviceHandle>, DiscoveryError> {
        let block_devices = ahci_block_devices(device, authority, config, ordinal)?;
        let mut handles = Vec::new();
        for block_device in block_devices {
            handles.push(service.register_device(block_device)?);
        }
        Ok(handles)
    }

    #[cfg(feature = "discovery-ahci")]
    fn ahci_block_devices(
        device: &PciDevice,
        authority: CapabilitySet,
        config: DiscoveryConfig,
        ordinal: u64,
    ) -> Result<Vec<Box<dyn mirage_block::BlockDevice>>, DiscoveryError> {
        use mirage_ahci::{AhciController, AhciControllerId, RealAhciBlockDevice};

        let mut controller = AhciController::from_pci_device(
            AhciControllerId::new(config.controller_id(ordinal)),
            device,
            authority,
            config.dma_region(ordinal),
        )
        .map_err(|_| DiscoveryError::DriverInitFailed {
            backend: StorageBackendKind::Ahci,
        })?;
        let ports = controller.probe_ports();
        let mut devices: Vec<Box<dyn mirage_block::BlockDevice>> = Vec::new();
        for port in ports {
            let identify =
                controller
                    .identify_device(port)
                    .map_err(|_| DiscoveryError::DriverInitFailed {
                        backend: StorageBackendKind::Ahci,
                    })?;
            devices.push(Box::new(RealAhciBlockDevice::new(
                controller.clone(),
                port,
                identify,
            )));
        }
        Ok(devices)
    }

    #[cfg(feature = "discovery-xhci")]
    fn init_xhci(
        device: &PciDevice,
        authority: CapabilitySet,
        config: DiscoveryConfig,
        ordinal: u64,
        registry: &mut StorageDeviceRegistry,
    ) -> Result<Vec<StorageDeviceHandle>, DiscoveryError> {
        let block_devices = xhci_block_devices(device, authority, config, ordinal)?;
        let mut handles = Vec::new();
        for block_device in block_devices {
            handles.push(registry.register(block_device)?);
        }
        Ok(handles)
    }

    #[cfg(feature = "discovery-xhci")]
    fn init_xhci_service(
        device: &PciDevice,
        authority: CapabilitySet,
        config: DiscoveryConfig,
        ordinal: u64,
        service: &mut StorageService,
    ) -> Result<Vec<StorageDeviceHandle>, DiscoveryError> {
        let block_devices = xhci_block_devices(device, authority, config, ordinal)?;
        let mut handles = Vec::new();
        for block_device in block_devices {
            handles.push(service.register_device(block_device)?);
        }
        Ok(handles)
    }

    #[cfg(feature = "discovery-xhci")]
    fn xhci_block_devices(
        device: &PciDevice,
        authority: CapabilitySet,
        config: DiscoveryConfig,
        ordinal: u64,
    ) -> Result<Vec<Box<dyn mirage_block::BlockDevice>>, DiscoveryError> {
        use mirage_block::{BlockDeviceId, BlockSize, SectorCount};
        use mirage_usb::{
            RealUsbBlockDevice, UsbControllerId, UsbDeviceClass, UsbScsiBlockInfo, XhciController,
        };

        let mut controller = XhciController::from_pci_device(
            UsbControllerId::new(config.controller_id(ordinal)),
            device,
            authority,
            config.dma_region(ordinal),
        )
        .map_err(|_| DiscoveryError::DriverInitFailed {
            backend: StorageBackendKind::UsbStorage,
        })?;
        let descriptors = controller
            .enumerate_usb()
            .map_err(|_| DiscoveryError::DriverInitFailed {
                backend: StorageBackendKind::UsbStorage,
            })?
            .to_vec();
        let mut devices: Vec<Box<dyn mirage_block::BlockDevice>> = Vec::new();
        for descriptor in descriptors {
            if descriptor.class != UsbDeviceClass::MassStorage {
                continue;
            }
            let info = UsbScsiBlockInfo {
                id: BlockDeviceId::new(
                    (config.controller_id(ordinal) << 8) | u64::from(descriptor.address.get()),
                ),
                block_size: BlockSize::new(512).map_err(|_| DiscoveryError::DriverInitFailed {
                    backend: StorageBackendKind::UsbStorage,
                })?,
                sectors: SectorCount::new(1024),
                read_only: false,
                write_cache: true,
            };
            devices.push(Box::new(RealUsbBlockDevice::new(
                controller.clone(),
                descriptor.address,
                info,
            )));
        }
        Ok(devices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirage_block::{BlockDeviceState, BlockSize};

    struct MockStorageDevice {
        info: BlockDeviceInfo,
        state: BlockDeviceState,
        storage: Vec<u8>,
        flushes: usize,
    }

    impl MockStorageDevice {
        fn new(id: u64, sectors: u64, block_size: u32) -> Self {
            let block_size = BlockSize::new(block_size).unwrap();
            Self {
                info: BlockDeviceInfo::new(
                    BlockDeviceId::new(id),
                    block_size,
                    SectorCount::new(sectors),
                    false,
                    true,
                ),
                state: BlockDeviceState::Online,
                storage: vec![0; sectors as usize * block_size.bytes_usize()],
                flushes: 0,
            }
        }

        fn with_pattern(id: u64) -> Self {
            let mut device = Self::new(id, 8, 4);
            device.storage[4..8].copy_from_slice(&[1, 2, 3, 4]);
            device
        }

        fn byte_bounds(&self, range: BlockRange) -> (usize, usize) {
            let start = range.start().get() as usize * self.info.block_size.bytes_usize();
            let len = range.byte_len(self.info.block_size).unwrap();
            (start, start + len)
        }
    }

    impl BlockDevice for MockStorageDevice {
        fn info(&self) -> BlockDeviceInfo {
            self.info
        }

        fn state(&self) -> BlockDeviceState {
            self.state
        }

        fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError> {
            self.validate_read(range, buffer)?;
            let (start, end) = self.byte_bounds(range);
            buffer.copy_from_slice(&self.storage[start..end]);
            Ok(())
        }

        fn write_blocks(&mut self, range: BlockRange, data: &[u8]) -> Result<(), BlockError> {
            self.validate_write(range, data)?;
            let (start, end) = self.byte_bounds(range);
            self.storage[start..end].copy_from_slice(data);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            self.state.ensure_available()?;
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn registering_devices_adds_handles_to_registry() {
        let mut service = StorageService::new();
        let handle = service
            .register_device(Box::new(MockStorageDevice::new(10, 16, 512)))
            .unwrap();

        assert_eq!(handle.id(), BlockDeviceId::new(10));
        assert_eq!(service.registry().len(), 1);
        assert_eq!(service.registry().handle(handle.id()), Some(&handle));
        assert_eq!(
            service.grant_read_only(handle.id()).unwrap().device_id(),
            handle.id()
        );
    }

    #[test]
    fn capability_checked_reads_return_device_data() {
        let mut service = StorageService::new();
        let handle = service
            .register_device(Box::new(MockStorageDevice::with_pattern(11)))
            .unwrap();
        let capability = service.grant_read_only(handle.id()).unwrap();
        let range = BlockRange::new(Lba::new(1), SectorCount::new(1));

        let data = service.read_vec(&capability, &handle, range).unwrap();

        assert_eq!(data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn read_is_denied_without_matching_capability() {
        let mut service = StorageService::new();
        let handle = service
            .register_device(Box::new(MockStorageDevice::with_pattern(12)))
            .unwrap();
        let other_capability = StorageCapability::read_only(BlockDeviceId::new(99));
        let range = BlockRange::new(Lba::new(1), SectorCount::new(1));

        let result = service.read_vec(&other_capability, &handle, range);

        assert_eq!(result, Err(StorageError::AccessDenied));
    }

    #[test]
    fn hotplug_and_remove_events_are_emitted_in_order() {
        let mut service = StorageService::new();
        let handle = service
            .register_device(Box::new(MockStorageDevice::new(13, 4, 512)))
            .unwrap();
        service.unregister_device(handle.id()).unwrap();

        assert_eq!(service.pending_events(), 2);
        assert_eq!(
            service.next_event(),
            Some(StorageEvent::DeviceHotplugged(handle.clone()))
        );
        assert_eq!(
            service.next_event(),
            Some(StorageEvent::DeviceRemoved(handle.id()))
        );
        assert_eq!(service.next_event(), None);
    }
}

/// Root filesystem selection parsed from the Mirage boot command line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootFsSelection {
    Auto,
    BuiltInQfs,
    Nvme0n1,
    Sata0,
    Named(&'static str),
}

/// Parse the storage-specific `root=` policy accepted by Mirage early boot.
pub fn parse_rootfs_selection(value: &'static str) -> Result<RootFsSelection, StorageError> {
    match value {
        "auto" => Ok(RootFsSelection::Auto),
        "builtin-qfs" => Ok(RootFsSelection::BuiltInQfs),
        "nvme0n1" => Ok(RootFsSelection::Nvme0n1),
        "sata0" => Ok(RootFsSelection::Sata0),
        "" => Err(StorageError::DeviceNotFound),
        other => Ok(RootFsSelection::Named(other)),
    }
}

#[cfg(test)]
mod rootfs_policy_tests {
    use super::*;

    #[test]
    fn root_parser_accepts_required_storage_policies() {
        assert_eq!(parse_rootfs_selection("auto"), Ok(RootFsSelection::Auto));
        assert_eq!(
            parse_rootfs_selection("builtin-qfs"),
            Ok(RootFsSelection::BuiltInQfs)
        );
        assert_eq!(
            parse_rootfs_selection("nvme0n1"),
            Ok(RootFsSelection::Nvme0n1)
        );
        assert_eq!(parse_rootfs_selection("sata0"), Ok(RootFsSelection::Sata0));
    }
}
