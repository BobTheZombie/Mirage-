#![no_std]
#![forbid(unsafe_code)]

//! AMDGPU driver service skeleton for Mirage.
//!
//! This crate models AMD GPU discovery and display bring-up as a supervised,
//! capability-mediated driver service. Default builds keep deterministic mock
//! resources while `hw-amdgpu` exposes a limited AMD PCI/MMIO/VRAM discovery path
//! without pretending that production modesetting or acceleration exists.

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
    Renoir,
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
            (AMD_VENDOR_ID, 0x1636) => Some(AmdGpuAsicFamily::Renoir),
            (AMD_VENDOR_ID, 0x73bf) => Some(AmdGpuAsicFamily::RDNA2),
            (AMD_VENDOR_ID, 0x744c) => Some(AmdGpuAsicFamily::RDNA3),
            (AMD_VENDOR_ID, _) => Some(AmdGpuAsicFamily::Unknown),
            _ => None,
        }
    }
}

/// Mock memory-mapped register aperture for the AMDGPU service.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct MockMmioRegion {
    pub base: u64,
    pub length: u64,
}

impl MockMmioRegion {
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
pub struct MockDisplayCore {
    pub outputs: Vec<DisplayOutput>,
    pub modes: Vec<DisplayMode>,
    pub active_output: u32,
    pub active_mode: DisplayMode,
}

impl MockDisplayCore {
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
pub struct MockFramebuffer {
    pub vram_base: u64,
    pub vram_length: u64,
    pub vram_object: u64,
    inner: Framebuffer,
}

impl MockFramebuffer {
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
pub struct MockHardwareResources {
    pub pci_device: u64,
    pub mmio: MockMmioRegion,
    pub dma_region: u64,
    pub irq_line: u16,
    pub vram_base: u64,
    pub vram_length: u64,
    pub vram_object: u64,
}

impl MockHardwareResources {
    pub const fn mock(pci: AmdGpuPciInfo) -> Self {
        Self {
            pci_device: pci.pci_device_key(),
            mmio: MockMmioRegion::mock(),
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
    #[cfg(feature = "hw-amdgpu")]
    Pci(mirage_pci::PciError),
    #[cfg(feature = "hw-amdgpu")]
    Hardware(mirage_hw::HardwareError),
    #[cfg(feature = "hw-amdgpu")]
    MissingBar {
        index: usize,
    },
    #[cfg(feature = "hw-amdgpu")]
    InvalidBarLayout,
}

impl From<CapabilityError> for AmdGpuError {
    fn from(error: CapabilityError) -> Self {
        Self::Capability(error)
    }
}

#[cfg(feature = "hw-amdgpu")]
impl From<mirage_pci::PciError> for AmdGpuError {
    fn from(error: mirage_pci::PciError) -> Self {
        Self::Pci(error)
    }
}

#[cfg(feature = "hw-amdgpu")]
impl From<mirage_hw::HardwareError> for AmdGpuError {
    fn from(error: mirage_hw::HardwareError) -> Self {
        Self::Hardware(error)
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
            #[cfg(feature = "hw-amdgpu")]
            AmdGpuError::Pci(_) => Self::AccessDenied,
            #[cfg(feature = "hw-amdgpu")]
            AmdGpuError::Hardware(mirage_hw::HardwareError::MissingCapability) => {
                Self::AccessDenied
            }
            #[cfg(feature = "hw-amdgpu")]
            AmdGpuError::Hardware(mirage_hw::HardwareError::InsufficientCapabilityRights) => {
                Self::InsufficientRights
            }
            #[cfg(feature = "hw-amdgpu")]
            AmdGpuError::Hardware(mirage_hw::HardwareError::RevokedCapability) => {
                Self::CapabilityRevoked
            }
            #[cfg(feature = "hw-amdgpu")]
            AmdGpuError::Hardware(_) => Self::AccessDenied,
            #[cfg(feature = "hw-amdgpu")]
            AmdGpuError::MissingBar { .. } | AmdGpuError::InvalidBarLayout => Self::AccessDenied,
        }
    }
}

/// Supervised mock AMDGPU device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockGpuDevice {
    info: GpuDeviceInfo,
    pci: AmdGpuPciInfo,
    asic_family: AmdGpuAsicFamily,
    resources: MockHardwareResources,
    authority: CapabilitySet,
    pub mmio: MockMmioRegion,
    pub firmware: AmdGpuFirmware,
    pub rings: Vec<AmdGpuRing>,
    pub display_core: MockDisplayCore,
    pub framebuffer: MockFramebuffer,
}

impl MockGpuDevice {
    /// Build a mock AMDGPU device after checking supervisor-granted authority.
    pub fn initialize(pci: AmdGpuPciInfo, authority: CapabilitySet) -> Result<Self, AmdGpuError> {
        // TODO: PCI enumeration should discover this device through supervisor policy.
        let asic_family = pci.asic_family().ok_or(AmdGpuError::UnsupportedPciDevice)?;
        let resources = MockHardwareResources::mock(pci);
        check_hardware_authority(&authority, resources)?;

        // TODO: BAR mapping should derive MMIO and VRAM apertures from PCI BARs.
        let mmio = resources.mmio;
        // TODO: AtomBIOS parsing should replace the fixed mock display tables.
        let display_core = MockDisplayCore::mock();
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
        let framebuffer = MockFramebuffer::mock(display_core.active_mode)?;

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

    pub const fn resources(&self) -> MockHardwareResources {
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
            Self::Renoir => "AMD Renoir APU",
            Self::RDNA2 => "Mock AMDGPU RDNA2",
            Self::RDNA3 => "Mock AMDGPU RDNA3",
        }
    }
}

impl GpuDevice for MockGpuDevice {
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

        self.framebuffer = MockFramebuffer::mock(mode).map_err(GpuError::from)?;
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
    resources: MockHardwareResources,
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

#[cfg(feature = "hw-amdgpu")]
pub mod hw_amdgpu {
    use super::*;
    use mirage_hw::{MmioRegion, PhysAddr, VirtAddr};
    use mirage_pci::{PciBarKind, PciDevice};

    /// Known AMD PCI IDs used by the hardware-gated AMDGPU skeleton.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
    pub struct AmdGpuDeviceIdEntry {
        pub device_id: u16,
        pub family: AmdGpuAsicFamily,
        pub marketing_name: &'static str,
    }

    /// Small explicit device-ID table for ASIC-family routing only.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
    pub struct AmdGpuDeviceIdTable {
        pub entries: &'static [AmdGpuDeviceIdEntry],
    }

    impl AmdGpuDeviceIdTable {
        pub const fn new(entries: &'static [AmdGpuDeviceIdEntry]) -> Self {
            Self { entries }
        }

        pub fn detect(self, pci: AmdGpuPciInfo) -> Option<AmdGpuAsicFamily> {
            if pci.vendor_id != AMD_VENDOR_ID {
                return None;
            }

            for entry in self.entries {
                if entry.device_id == pci.device_id {
                    return Some(entry.family);
                }
            }

            Some(AmdGpuAsicFamily::Unknown)
        }
    }

    pub const AMDGPU_DEVICE_IDS: AmdGpuDeviceIdTable = AmdGpuDeviceIdTable::new(&[
        AmdGpuDeviceIdEntry {
            device_id: 0x6780,
            family: AmdGpuAsicFamily::SouthernIslands,
            marketing_name: "Southern Islands",
        },
        AmdGpuDeviceIdEntry {
            device_id: 0x6640,
            family: AmdGpuAsicFamily::SeaIslands,
            marketing_name: "Sea Islands",
        },
        AmdGpuDeviceIdEntry {
            device_id: 0x67df,
            family: AmdGpuAsicFamily::Polaris,
            marketing_name: "Polaris",
        },
        AmdGpuDeviceIdEntry {
            device_id: 0x687f,
            family: AmdGpuAsicFamily::Vega,
            marketing_name: "Vega",
        },
        AmdGpuDeviceIdEntry {
            device_id: 0x731f,
            family: AmdGpuAsicFamily::Navi,
            marketing_name: "Navi",
        },
        AmdGpuDeviceIdEntry {
            device_id: 0x1636,
            family: AmdGpuAsicFamily::Renoir,
            marketing_name: "Renoir Radeon Vega Mobile",
        },
        AmdGpuDeviceIdEntry {
            device_id: 0x73bf,
            family: AmdGpuAsicFamily::RDNA2,
            marketing_name: "RDNA2",
        },
        AmdGpuDeviceIdEntry {
            device_id: 0x744c,
            family: AmdGpuAsicFamily::RDNA3,
            marketing_name: "RDNA3",
        },
    ]);

    /// Supervisor-provided AMDGPU BAR placement and mapping metadata.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
    pub struct AmdGpuBarLayout {
        pub mmio_bar: usize,
        pub mmio_phys_base: u64,
        pub mmio_virt_base: usize,
        pub mmio_length: usize,
        pub vram_bar: usize,
        pub vram_phys_base: u64,
        pub vram_length: u64,
        pub vram_object: u64,
    }

    impl AmdGpuBarLayout {
        pub const fn new(
            mmio_bar: usize,
            mmio_phys_base: u64,
            mmio_virt_base: usize,
            mmio_length: usize,
            vram_bar: usize,
            vram_phys_base: u64,
            vram_length: u64,
            vram_object: u64,
        ) -> Self {
            Self {
                mmio_bar,
                mmio_phys_base,
                mmio_virt_base,
                mmio_length,
                vram_bar,
                vram_phys_base,
                vram_length,
                vram_object,
            }
        }
    }

    /// Metadata for the VRAM aperture advertised by PCI BARs.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
    pub struct AmdGpuVramAperture {
        pub bar_index: usize,
        pub phys_base: u64,
        pub length: u64,
        pub memory_object: u64,
    }

    /// Safe wrapper around the `mirage-hw` MMIO mapping granted to AMDGPU.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
    pub struct AmdGpuMmioRegion {
        pub bar_index: usize,
        pub region: MmioRegion,
    }

    /// AMDGPU register aperture. This deliberately exposes only checked MMIO helpers.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
    pub struct AmdGpuRegisters {
        pub mmio: AmdGpuMmioRegion,
    }

    impl AmdGpuRegisters {
        pub fn read32(&self, offset: usize) -> Result<u32, AmdGpuError> {
            self.mmio.region.read32(offset).map_err(AmdGpuError::from)
        }

        pub fn write32(&mut self, offset: usize, value: u32) -> Result<(), AmdGpuError> {
            self.mmio
                .region
                .write32(offset, value)
                .map_err(AmdGpuError::from)
        }
    }

    /// Boot framebuffer preserved from firmware/bootloader handoff.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct AmdGpuBootFramebuffer {
        pub phys_base: u64,
        pub vram_offset: u64,
        pub framebuffer: Framebuffer,
    }

    impl AmdGpuBootFramebuffer {
        pub const fn new(phys_base: u64, vram_offset: u64, framebuffer: Framebuffer) -> Self {
            Self {
                phys_base,
                vram_offset,
                framebuffer,
            }
        }
    }

    /// Display metadata exposed to `GpuDevice` before real DC/DCE/DCN probing exists.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct AmdGpuDisplayInfo {
        pub outputs: Vec<DisplayOutput>,
        pub modes: Vec<DisplayMode>,
        pub active_output: u32,
        pub active_mode: DisplayMode,
        pub boot_framebuffer: Option<AmdGpuBootFramebuffer>,
    }

    impl AmdGpuDisplayInfo {
        pub fn new(
            outputs: Vec<DisplayOutput>,
            modes: Vec<DisplayMode>,
            active_output: u32,
            active_mode: DisplayMode,
            boot_framebuffer: Option<AmdGpuBootFramebuffer>,
        ) -> Self {
            Self {
                outputs,
                modes,
                active_output,
                active_mode,
                boot_framebuffer,
            }
        }

        pub fn boot_vga() -> Result<Self, AmdGpuError> {
            let active_mode = DisplayMode::new(1024, 768, 60_000, 32, PixelFormat::Xrgb);
            let framebuffer = Framebuffer::new(
                active_mode
                    .framebuffer_mode()
                    .map_err(AmdGpuError::InvalidMode)?,
            )
            .map_err(AmdGpuError::Framebuffer)?;
            Ok(Self::new(
                vec![DisplayOutput::new(0, "amdgpu-boot-output-0", true)],
                vec![active_mode],
                0,
                active_mode,
                Some(AmdGpuBootFramebuffer::new(0, 0, framebuffer)),
            ))
        }
    }

    /// Hardware-gated AMD PCI device skeleton exposed through `GpuDevice`.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct AmdGpuDevice {
        info: GpuDeviceInfo,
        pci: AmdGpuPciInfo,
        asic_family: AmdGpuAsicFamily,
        authority: CapabilitySet,
        pub registers: AmdGpuRegisters,
        pub vram: AmdGpuVramAperture,
        pub display: AmdGpuDisplayInfo,
        framebuffer: Framebuffer,
    }

    impl AmdGpuDevice {
        /// Construct a hardware-gated AMDGPU skeleton from a supervisor-enumerated PCI device.
        pub fn from_pci_device(
            pci_device: &PciDevice,
            authority: CapabilitySet,
            bar_layout: AmdGpuBarLayout,
            display_info: AmdGpuDisplayInfo,
        ) -> Result<Self, AmdGpuError> {
            let pci = Self::read_pci_info(pci_device)?;
            let asic_family = Self::detect_asic_family(pci)?;
            authority.check(
                CapabilityObject::PciDevice(pci.pci_device_key()),
                CapabilityRights::io().with(CapabilityRight::Read),
            )?;

            let registers = Self::map_bars(pci_device, bar_layout, &authority)?;
            let vram = Self::map_vram_aperture(pci_device, bar_layout, &authority)?;
            let framebuffer = match &display_info.boot_framebuffer {
                Some(boot) => boot.framebuffer.clone(),
                None => Framebuffer::new(
                    display_info
                        .active_mode
                        .framebuffer_mode()
                        .map_err(AmdGpuError::InvalidMode)?,
                )
                .map_err(AmdGpuError::Framebuffer)?,
            };

            // TODO: Implement DC/DCE/DCN modesetting rather than preserving boot modes only.
            // TODO: Add acceleration command submission behind explicit DMA capabilities.
            // TODO: Add graphics/compute rings with doorbells, fences, and scheduler ownership.
            // TODO: Load signed AMDGPU firmware modules through supervisor policy.
            // TODO: Add capability-scoped power management and clock control.
            // TODO: Replace polling stubs with interrupt-driven operation through IRQ IPC.

            let info = GpuDeviceInfo::new(
                GpuDeviceId::new(pci.pci_device_key()),
                "AMD",
                asic_family.model_name(),
                vec![
                    GpuMemoryRegion::new(
                        registers.mmio.region.base(),
                        registers.mmio.region.length() as u64,
                        GpuMemoryRegionKind::Mmio,
                    ),
                    GpuMemoryRegion::new(
                        vram.phys_base,
                        vram.length,
                        GpuMemoryRegionKind::Framebuffer,
                    ),
                ],
                display_info.outputs.clone(),
            );

            Ok(Self {
                info,
                pci,
                asic_family,
                authority,
                registers,
                vram,
                display: display_info,
                framebuffer,
            })
        }

        pub fn map_bars(
            pci_device: &PciDevice,
            bar_layout: AmdGpuBarLayout,
            authority: &CapabilitySet,
        ) -> Result<AmdGpuRegisters, AmdGpuError> {
            let mmio_bar = pci_device
                .bar(bar_layout.mmio_bar)
                .ok_or(AmdGpuError::MissingBar {
                    index: bar_layout.mmio_bar,
                })?;
            if mmio_bar.kind() == PciBarKind::IoPort || mmio_bar.base() != bar_layout.mmio_phys_base
            {
                return Err(AmdGpuError::InvalidBarLayout);
            }

            let region = MmioRegion::new(
                PhysAddr::new(bar_layout.mmio_phys_base),
                VirtAddr::new(bar_layout.mmio_virt_base),
                bar_layout.mmio_length,
                authority,
            )?;

            Ok(AmdGpuRegisters {
                mmio: AmdGpuMmioRegion {
                    bar_index: bar_layout.mmio_bar,
                    region,
                },
            })
        }

        pub fn read_pci_info(pci_device: &PciDevice) -> Result<AmdGpuPciInfo, AmdGpuError> {
            if !pci_device.is_amdgpu() {
                return Err(AmdGpuError::UnsupportedPciDevice);
            }

            let header = pci_device.header();
            Ok(AmdGpuPciInfo::new(
                header.vendor_id().get(),
                header.device_id().get(),
                header.subsystem_vendor_id().get(),
                header.subsystem_device_id().get(),
                header.revision_id(),
            ))
        }

        pub fn detect_asic_family(pci: AmdGpuPciInfo) -> Result<AmdGpuAsicFamily, AmdGpuError> {
            AMDGPU_DEVICE_IDS
                .detect(pci)
                .ok_or(AmdGpuError::UnsupportedPciDevice)
        }

        pub fn is_renoir_apu(pci_device: &PciDevice) -> Result<bool, AmdGpuError> {
            let pci = Self::read_pci_info(pci_device)?;
            Ok(Self::detect_asic_family(pci)? == AmdGpuAsicFamily::Renoir)
        }

        pub fn map_vram_aperture(
            pci_device: &PciDevice,
            bar_layout: AmdGpuBarLayout,
            authority: &CapabilitySet,
        ) -> Result<AmdGpuVramAperture, AmdGpuError> {
            let vram_bar = pci_device
                .bar(bar_layout.vram_bar)
                .ok_or(AmdGpuError::MissingBar {
                    index: bar_layout.vram_bar,
                })?;
            if vram_bar.kind() == PciBarKind::IoPort || vram_bar.base() != bar_layout.vram_phys_base
            {
                return Err(AmdGpuError::InvalidBarLayout);
            }
            if bar_layout.vram_length == 0 {
                return Err(AmdGpuError::InvalidBarLayout);
            }

            bar_layout
                .vram_phys_base
                .checked_add(bar_layout.vram_length)
                .ok_or(AmdGpuError::InvalidBarLayout)?;
            authority.check(
                CapabilityObject::MemoryObject(bar_layout.vram_object),
                CapabilityRights::read_write_io(),
            )?;

            Ok(AmdGpuVramAperture {
                bar_index: bar_layout.vram_bar,
                phys_base: bar_layout.vram_phys_base,
                length: bar_layout.vram_length,
                memory_object: bar_layout.vram_object,
            })
        }

        pub fn take_boot_framebuffer(&mut self) -> Option<AmdGpuBootFramebuffer> {
            self.display.boot_framebuffer.take()
        }

        pub fn provide_framebuffer(
            &self,
            output: u32,
            capability: Option<&GpuCapability>,
        ) -> Result<Framebuffer, GpuError> {
            self.ensure_access(output, capability)?;
            Ok(self.framebuffer.clone())
        }

        pub const fn pci_info(&self) -> AmdGpuPciInfo {
            self.pci
        }

        pub const fn asic_family(&self) -> AmdGpuAsicFamily {
            self.asic_family
        }

        fn ensure_output(&self, output: u32) -> Result<(), AmdGpuError> {
            if self
                .display
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
            self.authority
                .check(
                    CapabilityObject::PciDevice(self.pci.pci_device_key()),
                    CapabilityRights::io().with(CapabilityRight::Read),
                )
                .map_err(AmdGpuError::from)
                .map_err(GpuError::from)?;
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
            Ok(&self.display.modes)
        }

        fn set_mode(
            &mut self,
            output: u32,
            mode: DisplayMode,
            capability: Option<&GpuCapability>,
        ) -> Result<(), GpuError> {
            self.ensure_access(output, capability)?;
            if !self.display.modes.contains(&mode) {
                return Err(GpuError::ModeUnavailable);
            }

            // TODO: Program DC/DCE/DCN display engines instead of replacing a software buffer.
            self.framebuffer =
                Framebuffer::new(mode.framebuffer_mode().map_err(AmdGpuError::InvalidMode)?)
                    .map_err(AmdGpuError::Framebuffer)
                    .map_err(GpuError::from)?;
            self.display.active_output = output;
            self.display.active_mode = mode;
            Ok(())
        }

        fn framebuffer(
            &self,
            output: u32,
            capability: Option<&GpuCapability>,
        ) -> Result<Framebuffer, GpuError> {
            self.provide_framebuffer(output, capability)
        }
    }
}

#[cfg(feature = "hw-amdgpu")]
pub use hw_amdgpu::*;

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
        let resources = MockHardwareResources::mock(pci());
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

    fn device() -> MockGpuDevice {
        MockGpuDevice::initialize(pci(), full_authority()).unwrap()
    }

    fn display_capability(device: &MockGpuDevice) -> GpuCapability {
        GpuCapability::display(device.device_id(), MOCK_OUTPUT_ID)
    }

    #[cfg(feature = "hw-amdgpu")]
    fn hw_pci_device() -> mirage_pci::PciDevice {
        use mirage_pci::{PciAddress, PciClassCode, PciConfigSpace, PciDeviceId, PciVendorId};

        let mut config = PciConfigSpace::endpoint(
            PciVendorId::AMD,
            PciDeviceId::new(0x73bf),
            PciClassCode::from_raw(0x03, 0x00, 0x00),
            0x01,
        );
        config.write_u16(0x2c, AMD_VENDOR_ID).unwrap();
        config.write_u16(0x2e, 0x73bf).unwrap();
        config.write_u32(0x10, 0x8000_0008).unwrap();
        config.write_u32(0x24, 0xf000_0000).unwrap();
        mirage_pci::PciDevice::new(PciAddress::new(0, 4, 0).unwrap(), config).unwrap()
    }

    #[cfg(feature = "hw-amdgpu")]
    fn hw_bar_layout() -> AmdGpuBarLayout {
        AmdGpuBarLayout::new(
            5,
            0xf000_0000,
            0x1000_0000,
            0x1000,
            0,
            0x8000_0000,
            16 * 1024 * 1024,
            MOCK_VRAM_OBJECT,
        )
    }

    #[cfg(feature = "hw-amdgpu")]
    fn hw_authority() -> CapabilitySet {
        let pci = AmdGpuPciInfo::mock(0x73bf);
        CapabilitySet::from_capabilities(vec![
            Capability::new(
                CapabilityObject::PciDevice(pci.pci_device_key()),
                CapabilityRights::read_write_io(),
            ),
            Capability::new(
                CapabilityObject::MmioRegion {
                    base: 0xf000_0000,
                    length: 0x1000,
                },
                CapabilityRights::read_write_io(),
            ),
            Capability::new(
                CapabilityObject::MemoryObject(MOCK_VRAM_OBJECT),
                CapabilityRights::read_write_io(),
            ),
        ])
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
            AmdGpuPciInfo::mock(0x1636).asic_family(),
            Some(AmdGpuAsicFamily::Renoir)
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
        assert_eq!(device.mmio, MockMmioRegion::mock());
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
        let resources = MockHardwareResources::mock(pci());
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
            MockGpuDevice::initialize(pci(), missing_vram),
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

    #[cfg(feature = "hw-amdgpu")]
    #[test]
    fn hw_amdgpu_reads_pci_maps_bars_and_preserves_boot_framebuffer() {
        let pci_device = hw_pci_device();
        let pci_info = AmdGpuDevice::read_pci_info(&pci_device).unwrap();

        assert_eq!(pci_info.device_id, 0x73bf);
        assert_eq!(
            AmdGpuDevice::detect_asic_family(pci_info),
            Ok(AmdGpuAsicFamily::RDNA2)
        );

        let display_info = AmdGpuDisplayInfo::boot_vga().unwrap();
        let mut device = AmdGpuDevice::from_pci_device(
            &pci_device,
            hw_authority(),
            hw_bar_layout(),
            display_info,
        )
        .unwrap();

        assert_eq!(device.registers.mmio.region.base(), 0xf000_0000);
        assert_eq!(device.vram.phys_base, 0x8000_0000);
        assert!(device.take_boot_framebuffer().is_some());

        let capability = GpuCapability::display(device.info().id, 0);
        let framebuffer = device.provide_framebuffer(0, Some(&capability)).unwrap();
        assert_eq!(framebuffer.memory().len(), 1024 * 768 * 4);
    }
}
