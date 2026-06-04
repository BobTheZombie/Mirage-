#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

//! PCI identity, config-space parsing, and enumeration primitives for Mirage.
//!
//! The generic PCI model in this crate is deliberately mechanism-only: it knows
//! how to parse PCI configuration bytes, decode class codes, and enumerate an
//! abstract config-space provider. Architecture-specific access paths such as
//! x86_64 legacy `0xCF8` / `0xCFC` I/O ports live under [`arch`] so supervised
//! driver services can keep raw hardware authority behind capability boundaries.

extern crate alloc;

use alloc::vec::Vec;

pub mod arch;

/// Raw vendor identifier value used by PCI functions that are not present.
pub const PCI_VENDOR_ID_INVALID: u16 = 0xffff;

/// PCI vendor identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PciVendorId(u16);

impl PciVendorId {
    pub const INVALID: Self = Self(PCI_VENDOR_ID_INVALID);
    pub const AMD: Self = Self(0x1002);

    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u16 {
        self.0
    }

    pub const fn is_valid(self) -> bool {
        self.0 != PCI_VENDOR_ID_INVALID
    }
}

/// Backwards-compatible alias for older Mirage PCI code.
pub type VendorId = PciVendorId;

/// PCI device identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PciDeviceId(u16);

impl PciDeviceId {
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Backwards-compatible alias for older Mirage PCI code.
pub type DeviceId = PciDeviceId;

/// Bus/device/function tuple used to address a PCI function.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PciAddress {
    bus: u8,
    device: u8,
    function: u8,
}

impl PciAddress {
    pub const MAX_DEVICE: u8 = 31;
    pub const MAX_FUNCTION: u8 = 7;

    pub const fn new(bus: u8, device: u8, function: u8) -> Result<Self, PciError> {
        if device > Self::MAX_DEVICE {
            Err(PciError::InvalidDevice)
        } else if function > Self::MAX_FUNCTION {
            Err(PciError::InvalidFunction)
        } else {
            Ok(Self {
                bus,
                device,
                function,
            })
        }
    }

    pub const fn new_unchecked(bus: u8, device: u8, function: u8) -> Self {
        Self {
            bus,
            device,
            function,
        }
    }

    pub const fn bus(self) -> u8 {
        self.bus
    }

    pub const fn device(self) -> u8 {
        self.device
    }

    pub const fn function(self) -> u8 {
        self.function
    }
}

/// PCI base class code.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PciClass(u8);

impl PciClass {
    pub const MASS_STORAGE: Self = Self(0x01);
    pub const SERIAL_BUS: Self = Self(0x0c);
    pub const DISPLAY: Self = Self(0x03);

    pub const fn new(raw: u8) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

/// PCI subclass code scoped by [`PciClass`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PciSubclass(u8);

impl PciSubclass {
    pub const NVM: Self = Self(0x08);
    pub const SATA: Self = Self(0x06);
    pub const USB: Self = Self(0x03);
    pub const VGA: Self = Self(0x00);
    pub const DISPLAY_OTHER: Self = Self(0x80);

    pub const fn new(raw: u8) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

/// PCI programming-interface code scoped by class/subclass.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PciProgIf(u8);

impl PciProgIf {
    pub const NVME: Self = Self(0x02);
    pub const AHCI: Self = Self(0x01);
    pub const XHCI: Self = Self(0x30);

    pub const fn new(raw: u8) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

/// PCI class triple used for coarse driver matching.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PciClassCode {
    class: PciClass,
    subclass: PciSubclass,
    prog_if: PciProgIf,
}

impl PciClassCode {
    pub const fn new(class: PciClass, subclass: PciSubclass, prog_if: PciProgIf) -> Self {
        Self {
            class,
            subclass,
            prog_if,
        }
    }

    pub const fn from_raw(class: u8, subclass: u8, prog_if: u8) -> Self {
        Self::new(
            PciClass::new(class),
            PciSubclass::new(subclass),
            PciProgIf::new(prog_if),
        )
    }

    pub const fn class(self) -> PciClass {
        self.class
    }

    pub const fn subclass(self) -> PciSubclass {
        self.subclass
    }

    pub const fn prog_if(self) -> PciProgIf {
        self.prog_if
    }

    pub const fn is_nvme(self) -> bool {
        self.class.0 == PciClass::MASS_STORAGE.0
            && self.subclass.0 == PciSubclass::NVM.0
            && self.prog_if.0 == PciProgIf::NVME.0
    }

    pub const fn is_ahci(self) -> bool {
        self.class.0 == PciClass::MASS_STORAGE.0
            && self.subclass.0 == PciSubclass::SATA.0
            && self.prog_if.0 == PciProgIf::AHCI.0
    }

    pub const fn is_xhci(self) -> bool {
        self.class.0 == PciClass::SERIAL_BUS.0
            && self.subclass.0 == PciSubclass::USB.0
            && self.prog_if.0 == PciProgIf::XHCI.0
    }

    pub const fn is_display_controller(self) -> bool {
        self.class.0 == PciClass::DISPLAY.0
    }
}

/// PCI BAR address space type.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum PciBarKind {
    Memory32,
    Memory64,
    IoPort,
}

/// PCI base address register descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PciBar {
    index: u8,
    base: u64,
    length: Option<u64>,
    kind: PciBarKind,
    prefetchable: bool,
}

impl PciBar {
    pub const MAX_INDEX: u8 = 5;

    pub const fn new(
        index: u8,
        base: u64,
        length: u64,
        kind: PciBarKind,
    ) -> Result<Self, PciError> {
        if index > Self::MAX_INDEX {
            Err(PciError::InvalidBar)
        } else if length == 0 {
            Err(PciError::EmptyBar)
        } else {
            Ok(Self {
                index,
                base,
                length: Some(length),
                kind,
                prefetchable: false,
            })
        }
    }

    pub const fn from_config_raw(
        index: u8,
        low: u32,
        high: Option<u32>,
    ) -> Result<Option<Self>, PciError> {
        if index > Self::MAX_INDEX {
            return Err(PciError::InvalidBar);
        }
        if low == 0 {
            return Ok(None);
        }

        if (low & 0x1) == 0x1 {
            Ok(Some(Self {
                index,
                base: (low & 0xffff_fffc) as u64,
                length: None,
                kind: PciBarKind::IoPort,
                prefetchable: false,
            }))
        } else {
            let mem_type = (low >> 1) & 0b11;
            let prefetchable = (low & 0x8) != 0;
            match mem_type {
                0b00 => Ok(Some(Self {
                    index,
                    base: (low & 0xffff_fff0) as u64,
                    length: None,
                    kind: PciBarKind::Memory32,
                    prefetchable,
                })),
                0b10 => Ok(Some(Self {
                    index,
                    base: (((match high {
                        Some(value) => value,
                        None => 0,
                    }) as u64)
                        << 32)
                        | ((low & 0xffff_fff0) as u64),
                    length: None,
                    kind: PciBarKind::Memory64,
                    prefetchable,
                })),
                _ => Err(PciError::UnsupportedBarType),
            }
        }
    }

    pub const fn index(self) -> u8 {
        self.index
    }

    pub const fn base(self) -> u64 {
        self.base
    }

    pub const fn length(self) -> Option<u64> {
        self.length
    }

    pub const fn kind(self) -> PciBarKind {
        self.kind
    }

    pub const fn prefetchable(self) -> bool {
        self.prefetchable
    }
}

/// Parsed PCI configuration header for a type-0 endpoint function.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PciHeader {
    vendor_id: PciVendorId,
    device_id: PciDeviceId,
    command: u16,
    status: u16,
    revision_id: u8,
    class_code: PciClassCode,
    cache_line_size: u8,
    latency_timer: u8,
    header_type: u8,
    bist: u8,
    bars: [Option<PciBar>; 6],
    subsystem_vendor_id: PciVendorId,
    subsystem_device_id: PciDeviceId,
    interrupt_line: u8,
    interrupt_pin: u8,
}

impl PciHeader {
    pub fn parse(config: &PciConfigSpace) -> Result<Self, PciError> {
        let header_type = config.read_u8(0x0e)?;
        if (header_type & 0x7f) != 0x00 {
            return Err(PciError::UnsupportedHeaderType);
        }

        let mut bars = [None; 6];
        let mut index = 0;
        while index < bars.len() {
            let offset = 0x10 + (index * 4);
            let low = config.read_u32(offset as u16)?;
            let high = if index < 5 {
                Some(config.read_u32((offset + 4) as u16)?)
            } else {
                None
            };
            let bar = PciBar::from_config_raw(index as u8, low, high)?;
            bars[index] = bar;
            if matches!(bar, Some(decoded) if decoded.kind() == PciBarKind::Memory64) {
                index += 2;
            } else {
                index += 1;
            }
        }

        Ok(Self {
            vendor_id: PciVendorId::new(config.read_u16(0x00)?),
            device_id: PciDeviceId::new(config.read_u16(0x02)?),
            command: config.read_u16(0x04)?,
            status: config.read_u16(0x06)?,
            revision_id: config.read_u8(0x08)?,
            class_code: PciClassCode::from_raw(
                config.read_u8(0x0b)?,
                config.read_u8(0x0a)?,
                config.read_u8(0x09)?,
            ),
            cache_line_size: config.read_u8(0x0c)?,
            latency_timer: config.read_u8(0x0d)?,
            header_type,
            bist: config.read_u8(0x0f)?,
            bars,
            subsystem_vendor_id: PciVendorId::new(config.read_u16(0x2c)?),
            subsystem_device_id: PciDeviceId::new(config.read_u16(0x2e)?),
            interrupt_line: config.read_u8(0x3c)?,
            interrupt_pin: config.read_u8(0x3d)?,
        })
    }

    pub const fn vendor_id(self) -> PciVendorId {
        self.vendor_id
    }
    pub const fn device_id(self) -> PciDeviceId {
        self.device_id
    }
    pub const fn command(self) -> u16 {
        self.command
    }
    pub const fn status(self) -> u16 {
        self.status
    }
    pub const fn revision_id(self) -> u8 {
        self.revision_id
    }
    pub const fn class_code(self) -> PciClassCode {
        self.class_code
    }
    pub const fn cache_line_size(self) -> u8 {
        self.cache_line_size
    }
    pub const fn latency_timer(self) -> u8 {
        self.latency_timer
    }
    pub const fn header_type(self) -> u8 {
        self.header_type
    }
    pub const fn is_multi_function(self) -> bool {
        (self.header_type & 0x80) != 0
    }
    pub const fn bist(self) -> u8 {
        self.bist
    }
    pub const fn bars(self) -> [Option<PciBar>; 6] {
        self.bars
    }
    pub const fn bar(self, index: usize) -> Option<PciBar> {
        if index < self.bars.len() {
            self.bars[index]
        } else {
            None
        }
    }
    pub const fn subsystem_vendor_id(self) -> PciVendorId {
        self.subsystem_vendor_id
    }
    pub const fn subsystem_device_id(self) -> PciDeviceId {
        self.subsystem_device_id
    }
    pub const fn interrupt_line(self) -> u8 {
        self.interrupt_line
    }
    pub const fn interrupt_pin(self) -> u8 {
        self.interrupt_pin
    }
}

/// A complete 256-byte PCI configuration-space snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PciConfigSpace {
    bytes: [u8; Self::SIZE],
}

impl PciConfigSpace {
    pub const SIZE: usize = 256;

    pub const fn new(bytes: [u8; Self::SIZE]) -> Self {
        Self { bytes }
    }

    pub const fn empty() -> Self {
        let mut bytes = [0; Self::SIZE];
        bytes[0] = 0xff;
        bytes[1] = 0xff;
        Self { bytes }
    }

    pub fn endpoint(
        vendor_id: PciVendorId,
        device_id: PciDeviceId,
        class_code: PciClassCode,
        revision_id: u8,
    ) -> Self {
        let mut config = Self {
            bytes: [0; Self::SIZE],
        };
        config
            .write_u16(0x00, vendor_id.get())
            .expect("valid vendor offset");
        config
            .write_u16(0x02, device_id.get())
            .expect("valid device offset");
        config
            .write_u8(0x08, revision_id)
            .expect("valid revision offset");
        config
            .write_u8(0x09, class_code.prog_if().get())
            .expect("valid prog-if offset");
        config
            .write_u8(0x0a, class_code.subclass().get())
            .expect("valid subclass offset");
        config
            .write_u8(0x0b, class_code.class().get())
            .expect("valid class offset");
        config
    }

    pub const fn as_bytes(&self) -> &[u8; Self::SIZE] {
        &self.bytes
    }

    pub fn read_u8(&self, offset: u16) -> Result<u8, PciError> {
        self.bytes
            .get(offset as usize)
            .copied()
            .ok_or(PciError::InvalidConfigOffset)
    }

    pub fn read_u16(&self, offset: u16) -> Result<u16, PciError> {
        let lo = self.read_u8(offset)?;
        let hi = self.read_u8(offset + 1)?;
        Ok(u16::from_le_bytes([lo, hi]))
    }

    pub fn read_u32(&self, offset: u16) -> Result<u32, PciError> {
        let b0 = self.read_u8(offset)?;
        let b1 = self.read_u8(offset + 1)?;
        let b2 = self.read_u8(offset + 2)?;
        let b3 = self.read_u8(offset + 3)?;
        Ok(u32::from_le_bytes([b0, b1, b2, b3]))
    }

    pub fn write_u8(&mut self, offset: u16, value: u8) -> Result<(), PciError> {
        let byte = self
            .bytes
            .get_mut(offset as usize)
            .ok_or(PciError::InvalidConfigOffset)?;
        *byte = value;
        Ok(())
    }

    pub fn write_u16(&mut self, offset: u16, value: u16) -> Result<(), PciError> {
        let [b0, b1] = value.to_le_bytes();
        self.write_u8(offset, b0)?;
        self.write_u8(offset + 1, b1)
    }

    pub fn write_u32(&mut self, offset: u16, value: u32) -> Result<(), PciError> {
        let [b0, b1, b2, b3] = value.to_le_bytes();
        self.write_u8(offset, b0)?;
        self.write_u8(offset + 1, b1)?;
        self.write_u8(offset + 2, b2)?;
        self.write_u8(offset + 3, b3)
    }

    pub fn parse_header(&self) -> Result<PciHeader, PciError> {
        PciHeader::parse(self)
    }
}

/// Minimal PCI function descriptor surfaced by supervised enumeration.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PciDevice {
    address: PciAddress,
    header: PciHeader,
    config_space: PciConfigSpace,
}

impl PciDevice {
    pub fn new(address: PciAddress, config_space: PciConfigSpace) -> Result<Self, PciError> {
        let header = config_space.parse_header()?;
        if !header.vendor_id().is_valid() {
            return Err(PciError::DeviceNotPresent);
        }
        Ok(Self {
            address,
            header,
            config_space,
        })
    }

    pub const fn address(&self) -> PciAddress {
        self.address
    }
    pub const fn header(&self) -> PciHeader {
        self.header
    }
    pub const fn vendor_id(&self) -> PciVendorId {
        self.header.vendor_id()
    }
    pub const fn device_id(&self) -> PciDeviceId {
        self.header.device_id()
    }
    pub const fn class_code(&self) -> PciClassCode {
        self.header.class_code()
    }
    pub const fn revision_id(&self) -> u8 {
        self.header.revision_id()
    }
    pub const fn config_space(&self) -> &PciConfigSpace {
        &self.config_space
    }
    pub const fn bar(&self, index: usize) -> Option<PciBar> {
        self.header.bar(index)
    }

    pub const fn matches_id(&self, vendor_id: PciVendorId, device_id: PciDeviceId) -> bool {
        self.vendor_id().0 == vendor_id.0 && self.device_id().0 == device_id.0
    }

    pub const fn is_nvme(&self) -> bool {
        detection::is_nvme(self)
    }
    pub const fn is_ahci(&self) -> bool {
        detection::is_ahci(self)
    }
    pub const fn is_xhci(&self) -> bool {
        detection::is_xhci(self)
    }
    pub const fn is_amdgpu(&self) -> bool {
        detection::is_amdgpu(self)
    }
}

/// Backwards-compatible alias for older Mirage PCI code.
pub type PciFunction = PciDevice;

/// Detection helpers for routing devices to supervised driver services.
pub mod detection {
    use super::{PciClass, PciDevice, PciSubclass, PciVendorId};

    pub const fn is_nvme(device: &PciDevice) -> bool {
        device.class_code().is_nvme()
    }

    pub const fn is_ahci(device: &PciDevice) -> bool {
        device.class_code().is_ahci()
    }

    pub const fn is_xhci(device: &PciDevice) -> bool {
        device.class_code().is_xhci()
    }

    pub const fn is_amdgpu(device: &PciDevice) -> bool {
        device.vendor_id().get() == PciVendorId::AMD.get()
            && device.class_code().class().get() == PciClass::DISPLAY.get()
            && (device.class_code().subclass().get() == PciSubclass::VGA.get()
                || device.class_code().subclass().get() == PciSubclass::DISPLAY_OTHER.get())
    }
}

/// Abstract PCI config-space reader implemented by mock or hardware backends.
pub trait PciConfigAccess {
    fn read_u32(&self, address: PciAddress, offset: u16) -> Result<u32, PciError>;

    fn read_config_space(&self, address: PciAddress) -> Result<PciConfigSpace, PciError> {
        let mut config = PciConfigSpace::empty();
        let mut offset = 0u16;
        while (offset as usize) < PciConfigSpace::SIZE {
            config.write_u32(offset, self.read_u32(address, offset)?)?;
            offset += 4;
        }
        Ok(config)
    }
}

/// PCI bus snapshot created by enumeration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PciBus {
    bus: u8,
    devices: Vec<PciDevice>,
}

impl PciBus {
    pub const fn empty(bus: u8) -> Self {
        Self {
            bus,
            devices: Vec::new(),
        }
    }

    pub fn enumerate<A: PciConfigAccess>(bus: u8, access: &A) -> Result<Self, PciError> {
        let mut pci_bus = Self::empty(bus);
        for device in 0..=PciAddress::MAX_DEVICE {
            let function0 = PciAddress::new_unchecked(bus, device, 0);
            let vendor = read_vendor(access, function0)?;
            if !vendor.is_valid() {
                continue;
            }

            let config0 = access.read_config_space(function0)?;
            let header0 = config0.parse_header()?;
            pci_bus.devices.push(PciDevice::new(function0, config0)?);

            if header0.is_multi_function() {
                for function in 1..=PciAddress::MAX_FUNCTION {
                    let address = PciAddress::new_unchecked(bus, device, function);
                    if read_vendor(access, address)?.is_valid() {
                        pci_bus
                            .devices
                            .push(PciDevice::new(address, access.read_config_space(address)?)?);
                    }
                }
            }
        }
        Ok(pci_bus)
    }

    pub const fn bus(&self) -> u8 {
        self.bus
    }
    pub fn devices(&self) -> &[PciDevice] {
        &self.devices
    }
    pub fn into_devices(self) -> Vec<PciDevice> {
        self.devices
    }
}

fn read_vendor<A: PciConfigAccess>(
    access: &A,
    address: PciAddress,
) -> Result<PciVendorId, PciError> {
    Ok(PciVendorId::new(
        (access.read_u32(address, 0x00)? & 0xffff) as u16,
    ))
}

/// Mock config-space backend for tests and supervised discovery demos.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MockPciConfigAccess {
    functions: Vec<(PciAddress, PciConfigSpace)>,
}

impl MockPciConfigAccess {
    pub const fn new() -> Self {
        Self {
            functions: Vec::new(),
        }
    }

    pub fn add_function(&mut self, address: PciAddress, config_space: PciConfigSpace) {
        if let Some((_, existing)) = self
            .functions
            .iter_mut()
            .find(|(candidate, _)| *candidate == address)
        {
            *existing = config_space;
        } else {
            self.functions.push((address, config_space));
        }
    }

    pub fn with_function(mut self, address: PciAddress, config_space: PciConfigSpace) -> Self {
        self.add_function(address, config_space);
        self
    }
}

impl PciConfigAccess for MockPciConfigAccess {
    fn read_u32(&self, address: PciAddress, offset: u16) -> Result<u32, PciError> {
        if offset % 4 != 0 || offset as usize >= PciConfigSpace::SIZE {
            return Err(PciError::InvalidConfigOffset);
        }

        let config = self
            .functions
            .iter()
            .find(|(candidate, _)| *candidate == address)
            .map(|(_, config)| config);

        match config {
            Some(config) => config.read_u32(offset),
            None => Ok(0xffff_ffff),
        }
    }
}

/// Abstract PCI enumerator implemented by mock or feature-gated hardware code.
pub trait PciEnumerator {
    fn next_function(&mut self) -> Option<PciDevice>;
}

/// Errors returned while validating or enumerating PCI descriptors.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum PciError {
    InvalidDevice,
    InvalidFunction,
    InvalidBar,
    EmptyBar,
    UnsupportedBarType,
    UnsupportedHeaderType,
    InvalidConfigOffset,
    DeviceNotPresent,
    BackendDisabled,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(device: u8, function: u8) -> PciAddress {
        PciAddress::new(0, device, function).unwrap()
    }

    fn endpoint(vendor: u16, device: u16, class: u8, subclass: u8, prog_if: u8) -> PciConfigSpace {
        PciConfigSpace::endpoint(
            PciVendorId::new(vendor),
            PciDeviceId::new(device),
            PciClassCode::from_raw(class, subclass, prog_if),
            0x7a,
        )
    }

    #[test]
    fn parses_pci_header() {
        let mut config = endpoint(0x8086, 0x2922, 0x01, 0x06, 0x01);
        config.write_u16(0x04, 0x0007).unwrap();
        config.write_u16(0x06, 0x0010).unwrap();
        config.write_u16(0x2c, 0x1af4).unwrap();
        config.write_u16(0x2e, 0x1100).unwrap();
        config.write_u8(0x3c, 11).unwrap();
        config.write_u8(0x3d, 1).unwrap();

        let header = PciHeader::parse(&config).unwrap();

        assert_eq!(header.vendor_id(), PciVendorId::new(0x8086));
        assert_eq!(header.device_id(), PciDeviceId::new(0x2922));
        assert_eq!(header.command(), 0x0007);
        assert_eq!(header.status(), 0x0010);
        assert_eq!(header.revision_id(), 0x7a);
        assert!(header.class_code().is_ahci());
        assert_eq!(header.subsystem_vendor_id(), PciVendorId::new(0x1af4));
        assert_eq!(header.subsystem_device_id(), PciDeviceId::new(0x1100));
        assert_eq!(header.interrupt_line(), 11);
        assert_eq!(header.interrupt_pin(), 1);
    }

    #[test]
    fn decodes_class_codes() {
        let nvme = PciClassCode::from_raw(0x01, 0x08, 0x02);
        let ahci = PciClassCode::from_raw(0x01, 0x06, 0x01);
        let xhci = PciClassCode::from_raw(0x0c, 0x03, 0x30);

        assert!(nvme.is_nvme());
        assert!(ahci.is_ahci());
        assert!(xhci.is_xhci());
        assert!(!PciClassCode::from_raw(0x01, 0x06, 0x00).is_ahci());
    }

    #[test]
    fn enumerates_mock_bus() {
        let mut multifunction = endpoint(0x8086, 0x100e, 0x02, 0x00, 0x00);
        multifunction.write_u8(0x0e, 0x80).unwrap();

        let access = MockPciConfigAccess::new()
            .with_function(addr(1, 0), endpoint(0x144d, 0xa808, 0x01, 0x08, 0x02))
            .with_function(addr(2, 0), multifunction)
            .with_function(addr(2, 3), endpoint(0x8086, 0x2922, 0x01, 0x06, 0x01));

        let bus = PciBus::enumerate(0, &access).unwrap();

        assert_eq!(bus.bus(), 0);
        assert_eq!(bus.devices().len(), 3);
        assert_eq!(bus.devices()[0].address(), addr(1, 0));
        assert_eq!(bus.devices()[1].address(), addr(2, 0));
        assert_eq!(bus.devices()[2].address(), addr(2, 3));
    }

    #[test]
    fn decodes_bars() {
        let mut config = endpoint(0x1234, 0x5678, 0x01, 0x08, 0x02);
        config.write_u32(0x10, 0xfebc_0000).unwrap();
        config.write_u32(0x14, 0x0000_c004 | 0x8).unwrap();
        config.write_u32(0x18, 0x0000_0001).unwrap();
        config.write_u32(0x1c, 0x0000_d001).unwrap();

        let header = config.parse_header().unwrap();
        let bar0 = header.bar(0).unwrap();
        let bar1 = header.bar(1).unwrap();
        let bar3 = header.bar(3).unwrap();

        assert_eq!(bar0.kind(), PciBarKind::Memory32);
        assert_eq!(bar0.base(), 0xfebc_0000);
        assert!(!bar0.prefetchable());
        assert_eq!(bar1.kind(), PciBarKind::Memory64);
        assert_eq!(bar1.base(), 0x0000_0001_0000_c000);
        assert!(bar1.prefetchable());
        assert_eq!(header.bar(2), None);
        assert_eq!(bar3.kind(), PciBarKind::IoPort);
        assert_eq!(bar3.base(), 0x0000_d000);
    }

    #[test]
    fn detects_known_device_roles() {
        let nvme = PciDevice::new(addr(1, 0), endpoint(0x144d, 0xa808, 0x01, 0x08, 0x02)).unwrap();
        let ahci = PciDevice::new(addr(2, 0), endpoint(0x8086, 0x2922, 0x01, 0x06, 0x01)).unwrap();
        let xhci = PciDevice::new(addr(3, 0), endpoint(0x8086, 0x1e31, 0x0c, 0x03, 0x30)).unwrap();
        let amdgpu =
            PciDevice::new(addr(4, 0), endpoint(0x1002, 0x73bf, 0x03, 0x00, 0x00)).unwrap();

        assert!(nvme.is_nvme());
        assert!(ahci.is_ahci());
        assert!(xhci.is_xhci());
        assert!(amdgpu.is_amdgpu());
        assert!(!amdgpu.is_xhci());
    }
}
