#![no_std]
#![forbid(unsafe_code)]

//! Mock AMDGPU driver service skeleton for Mirage.
//!
//! This crate models AMD GPU discovery and display bring-up as a supervised,
//! capability-mediated driver service. It intentionally stops at mock resources
//! and framebuffer modes so Mirage can prove its GPU/display boundaries without
//! pretending that production hardware support exists.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use mirage_cap::{
    CapabilityError, CapabilityObject, CapabilityRight, CapabilityRights, CapabilitySet,
};
use mirage_fb::{Framebuffer, FramebufferError, PixelFormat};
use mirage_gpu::{
    DisplayMode, DisplayOutput, GpuCapability, GpuDevice, GpuDeviceId, GpuDeviceInfo, GpuError,
    GpuMemoryRegion, GpuMemoryRegionKind,
};

const AMD_VENDOR_ID: u16 = 0x1002;
const MOCK_OUTPUT_ID: u32 = 0;
const MOCK_MMIO_BASE: u64 = 0xf000_0000;
const MOCK_MMIO_LENGTH: u64 = 0x0010_0000;
const MOCK_VRAM_BASE: u64 = 0x8000_0000;
const MOCK_VRAM_LENGTH: u64 = 256 * 1024 * 1024;
const MOCK_DMA_REGION: u64 = 0x0a0d_0001;
const MOCK_IRQ_LINE: u16 = 44;
const MOCK_VRAM_OBJECT: u64 = 0x1002_0000_0001;

/// Mock AMD ASIC generations recognized by the Mirage AMDGPU skeleton.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum AmdGpuAsicFamily {
    Unknown,
    SouthernIslands,
    SeaIslands,
    Polaris,
    Vega,
    Navi,
    RDNA2,
    RDNA3,
}

/// Minimal PCI identity used by mock AMDGPU matching.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct AmdGpuPciInfo {
    pub vendor_id: u16,
    pub device_id: u16,
    pub subsystem_vendor_id: u16,
    pub subsystem_device_id: u16,
    pub revision_id: u8,
}

impl AmdGpuPciInfo {
    pub const fn new(
        vendor_id: u16,
        device_id: u16,
        subsystem_vendor_id: u16,
        subsystem_device_id: u16,
        revision_id: u8,
    ) -> Self {
        Self {
            vendor_id,
            device_id,
            subsystem_vendor_id,
            subsystem_device_id,
            revision_id,
        }
    }

    pub const fn mock(device_id: u16) -> Self {
        Self::new(AMD_VENDOR_ID, device_id, AMD_VENDOR_ID, device_id, 0)
    }

    pub const fn pci_device_key(self) -> u64 {
        ((self.vendor_id as u64) << 16) | self.device_id as u64
    }

    pub const fn asic_family(self) -> Option<AmdGpuAsicFamily> {
        match (self.vendor_id, self.device_id) {
            (AMD_VENDOR_ID, 0x6780) => Some(AmdGpuAsicFamily::SouthernIslands),
            (AMD_VENDOR_ID, 0x6640) => Some(AmdGpuAsicFamily::SeaIslands),
            (AMD_VENDOR_ID, 0x67df) => Some(AmdGpuAsicFamily::Polaris),
            (AMD_VENDOR_ID, 0x687f) => Some(AmdGpuAsicFamily::Vega),
            (AMD_VENDOR_ID, 0x731f) => Some(AmdGpuAsicFamily::Navi),
            (AMD_VENDOR_ID, 0x73bf) => Some(AmdGpuAsicFamily::RDNA2),
            (AMD_VENDOR_ID, 0x744c) => Some(AmdGpuAsicFamily::RDNA3),
            (AMD_VENDOR_ID, _) => Some(AmdGpuAsicFamily::Unknown),
            _ => None,
        }
    }
}

/// Mock memory-mapped register aperture for the AMDGPU service.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct AmdGpuMmio {
    pub base: u64,
    pub length: u64,
}

impl AmdGpuMmio {
    pub const fn mock() -> Self {
        Self {
            base: MOCK_MMIO_BASE,
            length: MOCK_MMIO_LENGTH,
        }
    }
}

/// Firmware manifest entry tracked by the mock driver service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdGpuFirmware {
    pub name: &'static str,
    pub loaded: bool,
}

impl AmdGpuFirmware {
    pub const fn mock() -> Self {
        Self {
            name: "mock-amdgpu-firmware.bin",
            loaded: false,
        }
    }
}

/// Mock command ring descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdGpuRing {
    pub name: &'static str,
    pub queue_capacity: usize,
    pub initialized: bool,
}

impl AmdGpuRing {
    pub const fn graphics() -> Self {
        Self {
            name: "gfx",
            queue_capacity: 256,
            initialized: false,
        }
    }
}

/// Mock display core state owned by the supervised AMDGPU service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdGpuDisplayCore {
    pub outputs: Vec<DisplayOutput>,
    pub modes: Vec<DisplayMode>,
    pub active_output: u32,
    pub active_mode: DisplayMode,
}

impl AmdGpuDisplayCore {
    pub fn mock() -> Self {
        let modes = vec![
            DisplayMode::new(1024, 768, 60_000, 32, PixelFormat::Xrgb),
            DisplayMode::new(1280, 720, 60_000, 32, PixelFormat::Xrgb),
            DisplayMode::new(1920, 1080, 60_000, 32, PixelFormat::Xrgb),
        ];

        Self {
            outputs: vec![DisplayOutput::new(MOCK_OUTPUT_ID, "amdgpu-mock-dp-0", true)],
            active_output: MOCK_OUTPUT_ID,
            active_mode: modes[0],
            modes,
        }
    }
}

/// Linear mock framebuffer backed by a capability-protected VRAM object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdGpuFramebuffer {
    pub vram_base: u64,
    pub vram_length: u64,
    pub vram_object: u64,
    inner: Framebuffer,
}

impl AmdGpuFramebuffer {
    pub fn mock(mode: DisplayMode) -> Result<Self, AmdGpuError> {
        let framebuffer_mode = mode.framebuffer_mode().map_err(AmdGpuError::InvalidMode)?;
        let inner = Framebuffer::new(framebuffer_mode).map_err(AmdGpuError::Framebuffer)?;
        Ok(Self {
            vram_base: MOCK_VRAM_BASE,
            vram_length: MOCK_VRAM_LENGTH,
            vram_object: MOCK_VRAM_OBJECT,
            inner,
        })
    }

    pub const fn inner(&self) -> &Framebuffer {
        &self.inner
    }

    pub fn clone_framebuffer(&self) -> Framebuffer {
        self.inner.clone()
    }
}

/// Capability-protected hardware resources required by the AMDGPU service.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct AmdGpuHardwareResources {
    pub pci_device: u64,
    pub mmio: AmdGpuMmio,
    pub dma_region: u64,
    pub irq_line: u16,
    pub vram_base: u64,
    pub vram_length: u64,
    pub vram_object: u64,
}

impl AmdGpuHardwareResources {
    pub const fn mock(pci: AmdGpuPciInfo) -> Self {
        Self {
            pci_device: pci.pci_device_key(),
            mmio: AmdGpuMmio::mock(),
            dma_region: MOCK_DMA_REGION,
            irq_line: MOCK_IRQ_LINE,
            vram_base: MOCK_VRAM_BASE,
            vram_length: MOCK_VRAM_LENGTH,
            vram_object: MOCK_VRAM_OBJECT,
        }
    }
}

/// Errors surfaced by the mock AMDGPU service skeleton.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AmdGpuError {
    Capability(CapabilityError),
    UnsupportedPciDevice,
    OutputNotFound,
    ModeUnavailable,
    InvalidMode(FramebufferError),
    Framebuffer(FramebufferError),
}

impl From<CapabilityError> for AmdGpuError {
    fn from(error: CapabilityError) -> Self {
        Self::Capability(error)
    }
}

impl From<AmdGpuError> for GpuError {
    fn from(error: AmdGpuError) -> Self {
        match error {
            AmdGpuError::Capability(CapabilityError::Missing) => Self::AccessDenied,
            AmdGpuError::Capability(CapabilityError::InsufficientRights) => {
                Self::InsufficientRights
            }
            AmdGpuError::Capability(CapabilityError::Revoked) => Self::CapabilityRevoked,
            AmdGpuError::OutputNotFound => Self::OutputNotFound,
            AmdGpuError::ModeUnavailable => Self::ModeUnavailable,
            AmdGpuError::InvalidMode(error) => Self::InvalidMode(error),
            AmdGpuError::Framebuffer(error) => Self::Framebuffer(error),
            AmdGpuError::UnsupportedPciDevice => Self::AccessDenied,
        }
    }
}

/// Supervised mock AMDGPU device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmdGpuDevice {
    info: GpuDeviceInfo,
    pci: AmdGpuPciInfo,
    asic_family: AmdGpuAsicFamily,
    resources: AmdGpuHardwareResources,
    authority: CapabilitySet,
    pub mmio: AmdGpuMmio,
    pub firmware: AmdGpuFirmware,
    pub rings: Vec<AmdGpuRing>,
    pub display_core: AmdGpuDisplayCore,
    pub framebuffer: AmdGpuFramebuffer,
}

impl AmdGpuDevice {
    /// Build a mock AMDGPU device after checking supervisor-granted authority.
    pub fn initialize(pci: AmdGpuPciInfo, authority: CapabilitySet) -> Result<Self, AmdGpuError> {
        // TODO: PCI enumeration should discover this device through supervisor policy.
        let asic_family = pci.asic_family().ok_or(AmdGpuError::UnsupportedPciDevice)?;
        let resources = AmdGpuHardwareResources::mock(pci);
        check_hardware_authority(&authority, resources)?;

        // TODO: BAR mapping should derive MMIO and VRAM apertures from PCI BARs.
        let mmio = resources.mmio;
        // TODO: AtomBIOS parsing should replace the fixed mock display tables.
        let display_core = AmdGpuDisplayCore::mock();
        // TODO: firmware loading must verify signed firmware modules before use.
        let firmware = AmdGpuFirmware::mock();
        // TODO: ring buffers need real doorbells, write pointers, and fences.
        let rings = vec![AmdGpuRing::graphics()];
        // TODO: command processor setup is not modeled beyond the mock ring descriptor.
        // TODO: display core initialization should program real DCN/DCE hardware blocks.
        // TODO: modesetting must be implemented by displayd policy over driver IPC.
        // TODO: VRAM manager should replace this single fixed mock framebuffer object.
        // TODO: GEM-like buffer abstraction should mediate client GPU buffers.
        // TODO: interrupt handling should route GPU IRQs through supervised IPC.
        // TODO: power management should be service-controlled and capability-scoped.
        // TODO: acceleration should remain behind explicit DMA/command capabilities.
        // TODO: Wayland compositor integration belongs above displayd, not in the kernel.
        let framebuffer = AmdGpuFramebuffer::mock(display_core.active_mode)?;

        let gpu_id = GpuDeviceId::new(resources.pci_device);
        let info = GpuDeviceInfo::new(
            gpu_id,
            "AMD",
            asic_family.model_name(),
            vec![
                GpuMemoryRegion::new(mmio.base, mmio.length, GpuMemoryRegionKind::Mmio),
                GpuMemoryRegion::new(
                    resources.vram_base,
                    resources.vram_length,
                    GpuMemoryRegionKind::Framebuffer,
                ),
                GpuMemoryRegion::new(resources.dma_region, 0, GpuMemoryRegionKind::Dma),
            ],
            display_core.outputs.clone(),
        );

        Ok(Self {
            info,
            pci,
            asic_family,
            resources,
            authority,
            mmio,
            firmware,
            rings,
            display_core,
            framebuffer,
        })
    }

    pub const fn pci_info(&self) -> AmdGpuPciInfo {
        self.pci
    }

    pub const fn asic_family(&self) -> AmdGpuAsicFamily {
        self.asic_family
    }

    pub const fn resources(&self) -> AmdGpuHardwareResources {
        self.resources
    }

    pub const fn active_mode(&self) -> DisplayMode {
        self.display_core.active_mode
    }

    pub const fn device_id(&self) -> GpuDeviceId {
        self.info.id
    }

    fn ensure_output(&self, output: u32) -> Result<(), AmdGpuError> {
        if self
            .display_core
            .outputs
            .iter()
            .any(|candidate| candidate.id == output)
        {
            Ok(())
        } else {
            Err(AmdGpuError::OutputNotFound)
        }
    }

    fn ensure_access(
        &self,
        output: u32,
        capability: Option<&GpuCapability>,
    ) -> Result<(), GpuError> {
        self.ensure_output(output).map_err(GpuError::from)?;
        check_hardware_authority(&self.authority, self.resources).map_err(GpuError::from)?;
        match capability {
            Some(capability) if capability.is_revoked() => Err(GpuError::CapabilityRevoked),
            Some(capability) if capability.device_id() != self.info.id => {
                Err(GpuError::AccessDenied)
            }
            Some(capability) => match capability.output_id() {
                Some(cap_output) if cap_output != output => Err(GpuError::AccessDenied),
                Some(_) | None => Ok(()),
            },
            None => Err(GpuError::AccessDenied),
        }
    }
}

impl AmdGpuAsicFamily {
    pub const fn model_name(self) -> &'static str {
        match self {
            Self::Unknown => "Mock AMDGPU Unknown ASIC",
            Self::SouthernIslands => "Mock AMDGPU Southern Islands",
            Self::SeaIslands => "Mock AMDGPU Sea Islands",
            Self::Polaris => "Mock AMDGPU Polaris",
            Self::Vega => "Mock AMDGPU Vega",
            Self::Navi => "Mock AMDGPU Navi",
            Self::RDNA2 => "Mock AMDGPU RDNA2",
            Self::RDNA3 => "Mock AMDGPU RDNA3",
        }
    }
}

impl GpuDevice for AmdGpuDevice {
    fn info(&self) -> &GpuDeviceInfo {
        &self.info
    }

    fn supported_modes(
        &self,
        output: u32,
        capability: Option<&GpuCapability>,
    ) -> Result<&[DisplayMode], GpuError> {
        self.ensure_access(output, capability)?;
        Ok(&self.display_core.modes)
    }

    fn set_mode(
        &mut self,
        output: u32,
        mode: DisplayMode,
        capability: Option<&GpuCapability>,
    ) -> Result<(), GpuError> {
        self.ensure_access(output, capability)?;
        if !self.display_core.modes.contains(&mode) {
            return Err(GpuError::ModeUnavailable);
        }

        self.framebuffer = AmdGpuFramebuffer::mock(mode).map_err(GpuError::from)?;
        self.display_core.active_output = output;
        self.display_core.active_mode = mode;
        Ok(())
    }

    fn framebuffer(
        &self,
        output: u32,
        capability: Option<&GpuCapability>,
    ) -> Result<Framebuffer, GpuError> {
        self.ensure_access(output, capability)?;
        if output != self.display_core.active_output {
            return Err(GpuError::OutputNotFound);
        }

        Ok(self.framebuffer.clone_framebuffer())
    }
}

fn check_hardware_authority(
    authority: &CapabilitySet,
    resources: AmdGpuHardwareResources,
) -> Result<(), AmdGpuError> {
    let io_read = CapabilityRights::io().with(CapabilityRight::Read);
    let read_write_io = CapabilityRights::read_write_io();

    authority.check(CapabilityObject::PciDevice(resources.pci_device), io_read)?;
    authority.check(
        CapabilityObject::MmioRegion {
            base: resources.mmio.base,
            length: resources.mmio.length,
        },
        read_write_io,
    )?;
    authority.check(
        CapabilityObject::DmaRegion(resources.dma_region),
        read_write_io,
    )?;
    authority.check(
        CapabilityObject::IrqLine(resources.irq_line),
        CapabilityRights::io(),
    )?;
    authority.check(
        CapabilityObject::MemoryObject(resources.vram_object),
        read_write_io,
    )?;
    Ok(())
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use mirage_cap::Capability;

    fn pci() -> AmdGpuPciInfo {
        AmdGpuPciInfo::mock(0x73bf)
    }

    fn full_authority() -> CapabilitySet {
        let resources = AmdGpuHardwareResources::mock(pci());
        CapabilitySet::from_capabilities(vec![
            Capability::new(
                CapabilityObject::PciDevice(resources.pci_device),
                CapabilityRights::read_write_io(),
            ),
            Capability::new(
                CapabilityObject::MmioRegion {
                    base: resources.mmio.base,
                    length: resources.mmio.length,
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
                CapabilityObject::MemoryObject(resources.vram_object),
                CapabilityRights::read_write_io(),
            ),
        ])
    }

    fn device() -> AmdGpuDevice {
        AmdGpuDevice::initialize(pci(), full_authority()).unwrap()
    }

    fn display_capability(device: &AmdGpuDevice) -> GpuCapability {
        GpuCapability::display(device.device_id(), MOCK_OUTPUT_ID)
    }

    #[test]
    fn mock_amd_pci_id_match() {
        assert_eq!(
            AmdGpuPciInfo::mock(0x6780).asic_family(),
            Some(AmdGpuAsicFamily::SouthernIslands)
        );
        assert_eq!(
            AmdGpuPciInfo::mock(0x73bf).asic_family(),
            Some(AmdGpuAsicFamily::RDNA2)
        );
        assert_eq!(
            AmdGpuPciInfo::mock(0xffff).asic_family(),
            Some(AmdGpuAsicFamily::Unknown)
        );
        assert_eq!(
            AmdGpuPciInfo::new(0x8086, 0x1234, 0, 0, 0).asic_family(),
            None
        );
    }

    #[test]
    fn mock_amdgpu_initialization() {
        let device = device();

        assert_eq!(device.asic_family(), AmdGpuAsicFamily::RDNA2);
        assert_eq!(device.info().vendor, "AMD");
        assert_eq!(device.info().outputs.len(), 1);
        assert_eq!(device.info().memory_regions.len(), 3);
        assert_eq!(device.mmio, AmdGpuMmio::mock());
    }

    #[test]
    fn framebuffer_exposure_through_gpu_device() {
        let device = device();
        let capability = display_capability(&device);

        let framebuffer = device
            .framebuffer(MOCK_OUTPUT_ID, Some(&capability))
            .unwrap();

        assert_eq!(
            framebuffer.mode(),
            device.active_mode().framebuffer_mode().unwrap()
        );
        assert_eq!(framebuffer.memory().len(), 1024 * 768 * 4);
    }

    #[test]
    fn capability_enforcement() {
        let resources = AmdGpuHardwareResources::mock(pci());
        let missing_vram = CapabilitySet::from_capabilities(vec![
            Capability::new(
                CapabilityObject::PciDevice(resources.pci_device),
                CapabilityRights::read_write_io(),
            ),
            Capability::new(
                CapabilityObject::MmioRegion {
                    base: resources.mmio.base,
                    length: resources.mmio.length,
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
        ]);

        assert_eq!(
            AmdGpuDevice::initialize(pci(), missing_vram),
            Err(AmdGpuError::Capability(CapabilityError::Missing))
        );

        let device = device();
        assert_eq!(
            device.supported_modes(MOCK_OUTPUT_ID, None),
            Err(GpuError::AccessDenied)
        );

        let mut revoked = display_capability(&device);
        revoked.revoke();
        assert_eq!(
            device.supported_modes(MOCK_OUTPUT_ID, Some(&revoked)),
            Err(GpuError::CapabilityRevoked)
        );
    }

    #[test]
    fn mock_mode_setting() {
        let mut device = device();
        let capability = display_capability(&device);
        let mode = DisplayMode::new(1280, 720, 60_000, 32, PixelFormat::Xrgb);

        assert_eq!(
            device.set_mode(MOCK_OUTPUT_ID, mode, Some(&capability)),
            Ok(())
        );
        assert_eq!(device.active_mode(), mode);
        assert_eq!(
            device
                .framebuffer(MOCK_OUTPUT_ID, Some(&capability))
                .unwrap()
                .memory()
                .len(),
            1280 * 720 * 4
        );
    }
}
