#![no_std]
#![forbid(unsafe_code)]

//! PCI identity and enumeration primitives for Mirage.
//!
//! PCI access in Mirage is capability-mediated and supervised. This crate only
//! defines no-std data types and traits that a mock or feature-gated hardware
//! enumerator can implement; it does not perform port I/O, MMIO config-space
//! access, or bus probing on its own.

/// PCI vendor identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct VendorId(u16);

impl VendorId {
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

/// PCI device identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DeviceId(u16);

impl DeviceId {
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

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

/// PCI class triple used for coarse driver matching.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PciClass {
    class: u8,
    subclass: u8,
    programming_interface: u8,
}

impl PciClass {
    pub const fn new(class: u8, subclass: u8, programming_interface: u8) -> Self {
        Self {
            class,
            subclass,
            programming_interface,
        }
    }

    pub const fn class(self) -> u8 {
        self.class
    }

    pub const fn subclass(self) -> u8 {
        self.subclass
    }

    pub const fn programming_interface(self) -> u8 {
        self.programming_interface
    }
}

/// Minimal PCI function descriptor surfaced by supervised enumeration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PciFunction {
    address: PciAddress,
    vendor_id: VendorId,
    device_id: DeviceId,
    class: PciClass,
    revision_id: u8,
}

impl PciFunction {
    pub const fn new(
        address: PciAddress,
        vendor_id: VendorId,
        device_id: DeviceId,
        class: PciClass,
        revision_id: u8,
    ) -> Self {
        Self {
            address,
            vendor_id,
            device_id,
            class,
            revision_id,
        }
    }

    pub const fn address(self) -> PciAddress {
        self.address
    }

    pub const fn vendor_id(self) -> VendorId {
        self.vendor_id
    }

    pub const fn device_id(self) -> DeviceId {
        self.device_id
    }

    pub const fn class(self) -> PciClass {
        self.class
    }

    pub const fn revision_id(self) -> u8 {
        self.revision_id
    }

    pub const fn matches_id(self, vendor_id: VendorId, device_id: DeviceId) -> bool {
        self.vendor_id.0 == vendor_id.0 && self.device_id.0 == device_id.0
    }
}

/// PCI base address register descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PciBar {
    index: u8,
    base: u64,
    length: u64,
    kind: PciBarKind,
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
                length,
                kind,
            })
        }
    }

    pub const fn index(self) -> u8 {
        self.index
    }

    pub const fn base(self) -> u64 {
        self.base
    }

    pub const fn length(self) -> u64 {
        self.length
    }

    pub const fn kind(self) -> PciBarKind {
        self.kind
    }
}

/// PCI BAR address space type.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum PciBarKind {
    Memory32,
    Memory64,
    IoPort,
}

/// Abstract PCI enumerator implemented by mock or feature-gated hardware code.
pub trait PciEnumerator {
    fn next_function(&mut self) -> Option<PciFunction>;
}

/// Errors returned while validating PCI descriptors.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum PciError {
    InvalidDevice,
    InvalidFunction,
    InvalidBar,
    EmptyBar,
    BackendDisabled,
}
