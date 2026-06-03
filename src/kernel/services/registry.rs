//! Fixed-capacity registry for L2-authorized user-space services.
//!
//! The registry maps stable service IDs (for example `displayd`) to the
//! process that currently owns the service endpoint.  It also records device
//! claims so the kernel can keep raw L1 device access scoped to authorized
//! driver daemons and route ordinary requests through service IPC.

use crate::kernel::device::{DeviceDescriptor, DeviceId, DeviceKind};
use crate::kernel::process::ProcessId;
use crate::subkernel::SecurityClass;

/// Maximum number of service endpoints tracked by the registry.
pub const MAX_SERVICE_REGISTRATIONS: usize = 12;

/// Maximum number of L1 devices that can be claimed by service daemons.
pub const MAX_DEVICE_CLAIMS: usize = 8;

/// Stable service identifiers used as IPC endpoints instead of exposing raw L1
/// device IDs to most tasks.
#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceId {
    Displayd = 1,
    /// Reserved IPC endpoint for POSIX-style socket operations routed through `networkd`.
    Networkd = 2,
    Inputd = 3,
    Storaged = 4,
    Usbd = 5,
    Nvmed = 6,
    Ahcid = 7,
    AmdgpuDisplayd = 8,
    SerialDriver = 0x100,
    TimerDriver = 0x101,
    BlockDriver = 0x102,
    FramebufferDriver = 0x103,
    GpuDriver = 0x104,
    NetworkDriver = 0x105,
    InputDriver = 0x106,
    SubkernelDriver = 0x107,
}

/// Stable service identifier used by socket syscalls when routing requests.
pub const NETWORK_SERVICE_ID: ServiceId = ServiceId::Networkd;

impl ServiceId {
    pub const fn raw(self) -> u64 {
        self as u64
    }

    pub const fn from_raw(raw: u64) -> Option<Self> {
        match raw {
            1 => Some(Self::Displayd),
            2 => Some(Self::Networkd),
            3 => Some(Self::Inputd),
            4 => Some(Self::Storaged),
            5 => Some(Self::Usbd),
            6 => Some(Self::Nvmed),
            7 => Some(Self::Ahcid),
            8 => Some(Self::AmdgpuDisplayd),
            0x100 => Some(Self::SerialDriver),
            0x101 => Some(Self::TimerDriver),
            0x102 => Some(Self::BlockDriver),
            0x103 => Some(Self::FramebufferDriver),
            0x104 => Some(Self::GpuDriver),
            0x105 => Some(Self::NetworkDriver),
            0x106 => Some(Self::InputDriver),
            0x107 => Some(Self::SubkernelDriver),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Displayd => "displayd",
            Self::Networkd => "networkd",
            Self::Inputd => "inputd",
            Self::Storaged => "storaged",
            Self::Usbd => "usbd",
            Self::Nvmed => "nvmed",
            Self::Ahcid => "ahcid",
            Self::AmdgpuDisplayd => "amdgpu-displayd",
            Self::SerialDriver => "driverd.serial",
            Self::TimerDriver => "driverd.timer",
            Self::BlockDriver => "driverd.block",
            Self::FramebufferDriver => "driverd.framebuffer",
            Self::GpuDriver => "driverd.gpu",
            Self::NetworkDriver => "driverd.network",
            Self::InputDriver => "driverd.input",
            Self::SubkernelDriver => "driverd.subkernel",
        }
    }

    pub const fn security_class(self) -> SecurityClass {
        match self {
            Self::Displayd
            | Self::Networkd
            | Self::Inputd
            | Self::Storaged
            | Self::Usbd
            | Self::Nvmed
            | Self::Ahcid
            | Self::AmdgpuDisplayd => SecurityClass::Internal,
            Self::SerialDriver
            | Self::BlockDriver
            | Self::FramebufferDriver
            | Self::GpuDriver
            | Self::NetworkDriver
            | Self::InputDriver => SecurityClass::Internal,
            Self::TimerDriver | Self::SubkernelDriver => SecurityClass::System,
        }
    }

    pub const fn for_device_kind(kind: DeviceKind) -> Self {
        match kind {
            DeviceKind::SerialConsole => Self::SerialDriver,
            DeviceKind::SystemTimer => Self::TimerDriver,
            DeviceKind::BlockStorage => Self::BlockDriver,
            DeviceKind::Framebuffer => Self::FramebufferDriver,
            DeviceKind::GpuCapability => Self::GpuDriver,
            DeviceKind::NetworkInterface => Self::NetworkDriver,
            DeviceKind::InputController => Self::InputDriver,
            DeviceKind::SubkernelControl => Self::SubkernelDriver,
        }
    }

    pub const fn can_claim_device_kind(self, kind: DeviceKind) -> bool {
        matches!(
            (self, kind),
            (Self::Displayd, DeviceKind::Framebuffer)
                | (Self::Displayd, DeviceKind::GpuCapability)
                | (Self::Networkd, DeviceKind::NetworkInterface)
                | (Self::Inputd, DeviceKind::InputController)
                | (Self::Storaged, DeviceKind::BlockStorage)
                | (Self::Usbd, DeviceKind::InputController)
                | (Self::Nvmed, DeviceKind::BlockStorage)
                | (Self::Ahcid, DeviceKind::BlockStorage)
                | (Self::AmdgpuDisplayd, DeviceKind::Framebuffer)
                | (Self::AmdgpuDisplayd, DeviceKind::GpuCapability)
                | (Self::SerialDriver, DeviceKind::SerialConsole)
                | (Self::TimerDriver, DeviceKind::SystemTimer)
                | (Self::BlockDriver, DeviceKind::BlockStorage)
                | (Self::FramebufferDriver, DeviceKind::Framebuffer)
                | (Self::GpuDriver, DeviceKind::GpuCapability)
                | (Self::NetworkDriver, DeviceKind::NetworkInterface)
                | (Self::InputDriver, DeviceKind::InputController)
                | (Self::SubkernelDriver, DeviceKind::SubkernelControl)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceRegistryError {
    Full,
    AlreadyRegistered,
    NotRegistered,
    DeviceAlreadyClaimed,
    DeviceNotClaimed,
    DeviceClassMismatch,
    NotOwner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceRegistration {
    pub service: ServiceId,
    pub owner: ProcessId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceClaim {
    pub service: ServiceId,
    pub owner: ProcessId,
    pub device: DeviceId,
}

#[derive(Clone, Copy)]
pub struct ServiceRegistry<const SERVICES: usize, const CLAIMS: usize> {
    services: [Option<ServiceRegistration>; SERVICES],
    claims: [Option<DeviceClaim>; CLAIMS],
}

impl<const SERVICES: usize, const CLAIMS: usize> ServiceRegistry<SERVICES, CLAIMS> {
    pub const fn new() -> Self {
        Self {
            services: [None; SERVICES],
            claims: [None; CLAIMS],
        }
    }

    pub fn reset(&mut self) {
        let mut idx = 0usize;
        while idx < SERVICES {
            self.services[idx] = None;
            idx += 1;
        }
        idx = 0;
        while idx < CLAIMS {
            self.claims[idx] = None;
            idx += 1;
        }
    }

    pub fn register(
        &mut self,
        service: ServiceId,
        owner: ProcessId,
    ) -> Result<(), ServiceRegistryError> {
        if let Some(idx) = self.find_service_slot(service) {
            if let Some(existing) = self.services[idx] {
                if existing.owner == owner {
                    return Ok(());
                }
                return Err(ServiceRegistryError::AlreadyRegistered);
            }
        }

        let slot = self
            .find_free_service_slot()
            .ok_or(ServiceRegistryError::Full)?;
        self.services[slot] = Some(ServiceRegistration { service, owner });
        Ok(())
    }

    pub fn owner(&self, service: ServiceId) -> Option<ProcessId> {
        self.find_service_slot(service)
            .and_then(|idx| self.services[idx].map(|registration| registration.owner))
    }

    pub fn claim_device(
        &mut self,
        service: ServiceId,
        owner: ProcessId,
        descriptor: DeviceDescriptor,
    ) -> Result<(), ServiceRegistryError> {
        if !service.can_claim_device_kind(descriptor.kind) {
            return Err(ServiceRegistryError::DeviceClassMismatch);
        }
        if self.owner(service) != Some(owner) {
            return Err(ServiceRegistryError::NotRegistered);
        }
        if let Some(claim) = self.claim_for_device(descriptor.id) {
            if claim.owner == owner && claim.service == service {
                return Ok(());
            }
            return Err(ServiceRegistryError::DeviceAlreadyClaimed);
        }
        let slot = self
            .find_free_claim_slot()
            .ok_or(ServiceRegistryError::Full)?;
        self.claims[slot] = Some(DeviceClaim {
            service,
            owner,
            device: descriptor.id,
        });
        Ok(())
    }

    pub fn release_device(
        &mut self,
        service: ServiceId,
        owner: ProcessId,
        device: DeviceId,
    ) -> Result<(), ServiceRegistryError> {
        let idx = self
            .find_claim_slot(device)
            .ok_or(ServiceRegistryError::DeviceNotClaimed)?;
        let claim = self.claims[idx].ok_or(ServiceRegistryError::DeviceNotClaimed)?;
        if claim.owner != owner || claim.service != service {
            return Err(ServiceRegistryError::NotOwner);
        }
        self.claims[idx] = None;
        Ok(())
    }

    pub fn claimed_by(&self, owner: ProcessId, device: DeviceId) -> bool {
        self.claim_for_device(device)
            .map(|claim| claim.owner == owner)
            .unwrap_or(false)
    }

    pub fn revoke_owner(&mut self, owner: ProcessId) {
        let mut idx = 0usize;
        while idx < SERVICES {
            if self.services[idx]
                .map(|registration| registration.owner == owner)
                .unwrap_or(false)
            {
                self.services[idx] = None;
            }
            idx += 1;
        }
        idx = 0;
        while idx < CLAIMS {
            if self.claims[idx]
                .map(|claim| claim.owner == owner)
                .unwrap_or(false)
            {
                self.claims[idx] = None;
            }
            idx += 1;
        }
    }

    fn claim_for_device(&self, device: DeviceId) -> Option<DeviceClaim> {
        self.find_claim_slot(device)
            .and_then(|idx| self.claims[idx])
    }

    fn find_service_slot(&self, service: ServiceId) -> Option<usize> {
        let mut idx = 0usize;
        while idx < SERVICES {
            if self.services[idx]
                .map(|registration| registration.service == service)
                .unwrap_or(false)
            {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    fn find_free_service_slot(&self) -> Option<usize> {
        let mut idx = 0usize;
        while idx < SERVICES {
            if self.services[idx].is_none() {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    fn find_claim_slot(&self, device: DeviceId) -> Option<usize> {
        let mut idx = 0usize;
        while idx < CLAIMS {
            if self.claims[idx]
                .map(|claim| claim.device == device)
                .unwrap_or(false)
            {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    fn find_free_claim_slot(&self) -> Option<usize> {
        let mut idx = 0usize;
        while idx < CLAIMS {
            if self.claims[idx].is_none() {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }
}
