#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;

/// Hardware or kernel object guarded by Mirage capability enforcement.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum CapabilityObject {
    PciDevice(u64),
    MmioRegion { base: u64, length: u64 },
    DmaRegion(u64),
    DmaBuffer { base: u64, length: u64 },
    MemoryObject(u64),
    VramRegion { base: u64, length: u64 },
    Framebuffer { base: u64, length: u64 },
    IrqLine(u16),
    IpcEndpoint(u64),
    HotplugController(u64),
    BlockDeviceRegistry,
    DisplayRegistry,
}

/// Fine-grained operation right attached to a capability object.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum CapabilityRight {
    Read,
    Write,
    Send,
    Receive,
    Control,
    Io,
}

impl CapabilityRight {
    const fn bit(self) -> u16 {
        match self {
            Self::Read => 1 << 0,
            Self::Write => 1 << 1,
            Self::Send => 1 << 2,
            Self::Receive => 1 << 3,
            Self::Control => 1 << 4,
            Self::Io => 1 << 5,
        }
    }
}

/// Compact rights mask used by supervised services and drivers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CapabilityRights(u16);

impl CapabilityRights {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn io() -> Self {
        Self(CapabilityRight::Io.bit() | CapabilityRight::Control.bit())
    }

    pub const fn read_write_io() -> Self {
        Self(
            CapabilityRight::Read.bit()
                | CapabilityRight::Write.bit()
                | CapabilityRight::Control.bit()
                | CapabilityRight::Io.bit(),
        )
    }

    pub const fn ipc() -> Self {
        Self(CapabilityRight::Send.bit() | CapabilityRight::Receive.bit())
    }

    pub const fn with(self, right: CapabilityRight) -> Self {
        Self(self.0 | right.bit())
    }

    pub const fn contains(self, right: CapabilityRight) -> bool {
        (self.0 & right.bit()) == right.bit()
    }

    pub const fn contains_all(self, requested: Self) -> bool {
        (self.0 & requested.0) == requested.0
    }
}

/// A supervisor-issued capability token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capability {
    object: CapabilityObject,
    rights: CapabilityRights,
    revoked: bool,
}

impl Capability {
    pub const fn new(object: CapabilityObject, rights: CapabilityRights) -> Self {
        Self {
            object,
            rights,
            revoked: false,
        }
    }

    pub const fn object(&self) -> CapabilityObject {
        self.object
    }

    pub const fn rights(&self) -> CapabilityRights {
        self.rights
    }

    pub const fn is_revoked(&self) -> bool {
        self.revoked
    }

    pub fn revoke(&mut self) {
        self.revoked = true;
    }

    pub fn permits(
        &self,
        object: CapabilityObject,
        rights: CapabilityRights,
    ) -> Result<(), CapabilityError> {
        if self.revoked {
            return Err(CapabilityError::Revoked);
        }

        if self.object != object {
            return Err(CapabilityError::Missing);
        }

        if self.rights.contains_all(rights) {
            Ok(())
        } else {
            Err(CapabilityError::InsufficientRights)
        }
    }
}

/// Small service-local capability set used by mock driver services.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilitySet {
    capabilities: Vec<Capability>,
}

impl CapabilitySet {
    pub const fn new() -> Self {
        Self {
            capabilities: Vec::new(),
        }
    }

    pub fn from_capabilities(capabilities: Vec<Capability>) -> Self {
        Self { capabilities }
    }

    pub fn grant(&mut self, capability: Capability) {
        self.capabilities.push(capability);
    }

    pub fn check(
        &self,
        object: CapabilityObject,
        rights: CapabilityRights,
    ) -> Result<(), CapabilityError> {
        let mut saw_revoked = false;
        let mut saw_object = false;

        for capability in &self.capabilities {
            if capability.object() != object {
                continue;
            }

            saw_object = true;
            match capability.permits(object, rights) {
                Ok(()) => return Ok(()),
                Err(CapabilityError::Revoked) => saw_revoked = true,
                Err(CapabilityError::InsufficientRights) => {}
                Err(CapabilityError::Missing) => {}
            }
        }

        if saw_revoked {
            Err(CapabilityError::Revoked)
        } else if saw_object {
            Err(CapabilityError::InsufficientRights)
        } else {
            Err(CapabilityError::Missing)
        }
    }
}

/// Capability validation failures reported before privileged operations execute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityError {
    Missing,
    InsufficientRights,
    Revoked,
}
