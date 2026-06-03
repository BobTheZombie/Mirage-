#![no_std]
#![forbid(unsafe_code)]

//! Capability-mediated GPU and display service primitives for Mirage.
//!
//! This crate models the early Mirage graphics boundary without pretending to
//! provide production hardware support. The long-term native graphics stack is:
//!
//! ```text
//! GPU module/service
//!     -> displayd
//!     -> Wayland compositor
//!     -> Wayland clients
//! ```
//!
//! X11 is intentionally not a base dependency for Mirage graphics. XWayland may
//! be layered on later as an optional compatibility service, but the native base
//! remains GPU module/service → `displayd` → Wayland compositor → Wayland
//! clients.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use mirage_cap::{
    Capability, CapabilityError, CapabilityObject, CapabilityRight, CapabilityRights,
};
use mirage_fb::{Framebuffer, FramebufferError, FramebufferMode, PixelFormat};

/// Stable supervisor-visible identifier for a GPU device.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct GpuDeviceId(u64);

impl GpuDeviceId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Static description of a registered GPU device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuDeviceInfo {
    pub id: GpuDeviceId,
    pub vendor: &'static str,
    pub model: &'static str,
    pub memory_regions: Vec<GpuMemoryRegion>,
    pub outputs: Vec<DisplayOutput>,
}

impl GpuDeviceInfo {
    pub fn new(
        id: GpuDeviceId,
        vendor: &'static str,
        model: &'static str,
        memory_regions: Vec<GpuMemoryRegion>,
        outputs: Vec<DisplayOutput>,
    ) -> Self {
        Self {
            id,
            vendor,
            model,
            memory_regions,
            outputs,
        }
    }
}

/// A display connector or virtual output exported through `displayd` policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayOutput {
    pub id: u32,
    pub name: &'static str,
    pub connected: bool,
}

impl DisplayOutput {
    pub const fn new(id: u32, name: &'static str, connected: bool) -> Self {
        Self {
            id,
            name,
            connected,
        }
    }
}

/// Display mode supported by a GPU output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct DisplayMode {
    pub width: usize,
    pub height: usize,
    pub refresh_millihertz: u32,
    pub bits_per_pixel: usize,
    pub pixel_format: PixelFormat,
}

impl DisplayMode {
    pub const fn new(
        width: usize,
        height: usize,
        refresh_millihertz: u32,
        bits_per_pixel: usize,
        pixel_format: PixelFormat,
    ) -> Self {
        Self {
            width,
            height,
            refresh_millihertz,
            bits_per_pixel,
            pixel_format,
        }
    }

    pub const fn framebuffer_mode(self) -> Result<FramebufferMode, FramebufferError> {
        let bytes_per_pixel = self.bits_per_pixel / 8;
        let pitch = match self.width.checked_mul(bytes_per_pixel) {
            Some(value) => value,
            None => return Err(FramebufferError::SizeOverflow),
        };

        FramebufferMode::new(
            self.width,
            self.height,
            pitch,
            self.bits_per_pixel,
            self.pixel_format,
        )
    }
}

/// GPU-visible memory exposed only through scoped capabilities.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct GpuMemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: GpuMemoryRegionKind,
}

impl GpuMemoryRegion {
    pub const fn new(base: u64, length: u64, kind: GpuMemoryRegionKind) -> Self {
        Self { base, length, kind }
    }
}

/// Purpose of a GPU memory region.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum GpuMemoryRegionKind {
    Mmio,
    Framebuffer,
    Dma,
}

/// Supervisor-issued authority for GPU/display operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuCapability {
    device_id: GpuDeviceId,
    output_id: Option<u32>,
    inner: Capability,
}

impl GpuCapability {
    /// Grants full device control. This is suitable for a GPU service, not for
    /// ordinary Wayland clients.
    pub const fn device(device_id: GpuDeviceId) -> Self {
        Self {
            device_id,
            output_id: None,
            inner: Capability::new(
                CapabilityObject::PciDevice(device_id.raw()),
                CapabilityRights::read_write_io(),
            ),
        }
    }

    /// Grants display-output control, the authority typically held by
    /// `displayd` while configuring a connector and publishing a framebuffer.
    pub const fn display(device_id: GpuDeviceId, output_id: u32) -> Self {
        Self {
            device_id,
            output_id: Some(output_id),
            inner: Capability::new(
                Self::display_object(device_id, output_id),
                CapabilityRights::read_write_io(),
            ),
        }
    }

    pub const fn device_id(&self) -> GpuDeviceId {
        self.device_id
    }

    pub const fn output_id(&self) -> Option<u32> {
        self.output_id
    }

    pub const fn is_revoked(&self) -> bool {
        self.inner.is_revoked()
    }

    pub fn revoke(&mut self) {
        self.inner.revoke();
    }

    fn permits_display(&self, device_id: GpuDeviceId, output_id: u32) -> Result<(), GpuError> {
        if self.device_id != device_id {
            return Err(GpuError::AccessDenied);
        }

        let object = match self.output_id {
            Some(cap_output_id) if cap_output_id == output_id => {
                Self::display_object(device_id, output_id)
            }
            Some(_) => return Err(GpuError::AccessDenied),
            None => CapabilityObject::PciDevice(device_id.raw()),
        };

        let rights = CapabilityRights::empty()
            .with(CapabilityRight::Read)
            .with(CapabilityRight::Write)
            .with(CapabilityRight::Control)
            .with(CapabilityRight::Io);

        self.inner.permits(object, rights).map_err(GpuError::from)
    }

    const fn display_object(device_id: GpuDeviceId, output_id: u32) -> CapabilityObject {
        CapabilityObject::IpcEndpoint((device_id.raw() << 32) | output_id as u64)
    }
}

/// GPU/display service failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GpuError {
    AccessDenied,
    CapabilityRevoked,
    InsufficientRights,
    OutputNotFound,
    ModeUnavailable,
    InvalidMode(FramebufferError),
    Framebuffer(FramebufferError),
}

impl From<CapabilityError> for GpuError {
    fn from(error: CapabilityError) -> Self {
        match error {
            CapabilityError::Missing => Self::AccessDenied,
            CapabilityError::InsufficientRights => Self::InsufficientRights,
            CapabilityError::Revoked => Self::CapabilityRevoked,
        }
    }
}

impl From<FramebufferError> for GpuError {
    fn from(error: FramebufferError) -> Self {
        Self::Framebuffer(error)
    }
}

/// Device-side interface implemented by GPU modules/services.
pub trait GpuDevice {
    fn info(&self) -> &GpuDeviceInfo;

    fn supported_modes(
        &self,
        output: u32,
        capability: Option<&GpuCapability>,
    ) -> Result<&[DisplayMode], GpuError>;

    fn set_mode(
        &mut self,
        output: u32,
        mode: DisplayMode,
        capability: Option<&GpuCapability>,
    ) -> Result<(), GpuError>;

    fn framebuffer(
        &self,
        output: u32,
        capability: Option<&GpuCapability>,
    ) -> Result<Framebuffer, GpuError>;
}

/// Supervisor-facing `displayd` facade over a GPU device.
pub struct DisplayService<D: GpuDevice> {
    device: D,
}

impl<D: GpuDevice> DisplayService<D> {
    pub const fn new(device: D) -> Self {
        Self { device }
    }

    pub const fn device(&self) -> &D {
        &self.device
    }

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    pub fn into_inner(self) -> D {
        self.device
    }

    pub fn modes(
        &self,
        output: u32,
        capability: Option<&GpuCapability>,
    ) -> Result<&[DisplayMode], GpuError> {
        self.device.supported_modes(output, capability)
    }

    pub fn set_mode(
        &mut self,
        output: u32,
        mode: DisplayMode,
        capability: Option<&GpuCapability>,
    ) -> Result<(), GpuError> {
        self.device.set_mode(output, mode, capability)
    }

    pub fn framebuffer(
        &self,
        output: u32,
        capability: Option<&GpuCapability>,
    ) -> Result<Framebuffer, GpuError> {
        self.device.framebuffer(output, capability)
    }
}

impl DisplayService<Box<dyn GpuDevice>> {
    pub fn boxed(device: Box<dyn GpuDevice>) -> Self {
        Self { device }
    }
}

impl GpuDevice for Box<dyn GpuDevice> {
    fn info(&self) -> &GpuDeviceInfo {
        self.as_ref().info()
    }

    fn supported_modes(
        &self,
        output: u32,
        capability: Option<&GpuCapability>,
    ) -> Result<&[DisplayMode], GpuError> {
        self.as_ref().supported_modes(output, capability)
    }

    fn set_mode(
        &mut self,
        output: u32,
        mode: DisplayMode,
        capability: Option<&GpuCapability>,
    ) -> Result<(), GpuError> {
        self.as_mut().set_mode(output, mode, capability)
    }

    fn framebuffer(
        &self,
        output: u32,
        capability: Option<&GpuCapability>,
    ) -> Result<Framebuffer, GpuError> {
        self.as_ref().framebuffer(output, capability)
    }
}

/// Mock GPU for architecture and service-boundary tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockGpuDevice {
    info: GpuDeviceInfo,
    modes: Vec<DisplayMode>,
    active_output: u32,
    active_mode: DisplayMode,
    framebuffer: Framebuffer,
}

impl MockGpuDevice {
    pub fn new(device_id: GpuDeviceId) -> Self {
        let output_id = 0;
        let output = DisplayOutput::new(output_id, "mock-output-0", true);
        let modes = vec![
            DisplayMode::new(640, 480, 60_000, 32, PixelFormat::Xrgb),
            DisplayMode::new(800, 600, 60_000, 32, PixelFormat::Xrgb),
            DisplayMode::new(1024, 768, 60_000, 32, PixelFormat::Xrgb),
        ];
        let active_mode = modes[0];
        let framebuffer = Framebuffer::new(
            active_mode
                .framebuffer_mode()
                .expect("built-in mock mode must be valid"),
        )
        .expect("built-in mock framebuffer mode must allocate");

        Self {
            info: GpuDeviceInfo::new(
                device_id,
                "Mirage",
                "Mock GPU",
                vec![GpuMemoryRegion::new(
                    0x1000_0000,
                    framebuffer.memory().len() as u64,
                    GpuMemoryRegionKind::Framebuffer,
                )],
                vec![output],
            ),
            modes,
            active_output: output_id,
            active_mode,
            framebuffer,
        }
    }

    pub const fn active_mode(&self) -> DisplayMode {
        self.active_mode
    }

    fn ensure_output(&self, output: u32) -> Result<(), GpuError> {
        if self
            .info
            .outputs
            .iter()
            .any(|candidate| candidate.id == output)
        {
            Ok(())
        } else {
            Err(GpuError::OutputNotFound)
        }
    }

    fn ensure_access(
        &self,
        output: u32,
        capability: Option<&GpuCapability>,
    ) -> Result<(), GpuError> {
        self.ensure_output(output)?;
        match capability {
            Some(capability) => capability.permits_display(self.info.id, output),
            None => Err(GpuError::AccessDenied),
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
        Ok(&self.modes)
    }

    fn set_mode(
        &mut self,
        output: u32,
        mode: DisplayMode,
        capability: Option<&GpuCapability>,
    ) -> Result<(), GpuError> {
        self.ensure_access(output, capability)?;
        if !self.modes.contains(&mode) {
            return Err(GpuError::ModeUnavailable);
        }

        let framebuffer_mode = mode.framebuffer_mode().map_err(GpuError::InvalidMode)?;
        self.framebuffer = Framebuffer::new(framebuffer_mode).map_err(GpuError::Framebuffer)?;
        self.active_output = output;
        self.active_mode = mode;
        Ok(())
    }

    fn framebuffer(
        &self,
        output: u32,
        capability: Option<&GpuCapability>,
    ) -> Result<Framebuffer, GpuError> {
        self.ensure_access(output, capability)?;
        if output != self.active_output {
            return Err(GpuError::OutputNotFound);
        }

        Ok(self.framebuffer.clone())
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE_ID: GpuDeviceId = GpuDeviceId::new(7);
    const OUTPUT_ID: u32 = 0;

    fn service() -> DisplayService<MockGpuDevice> {
        DisplayService::new(MockGpuDevice::new(DEVICE_ID))
    }

    fn display_capability() -> GpuCapability {
        GpuCapability::display(DEVICE_ID, OUTPUT_ID)
    }

    #[test]
    fn mock_gpu_lists_supported_modes() {
        let service = service();
        let capability = display_capability();

        let modes = service.modes(OUTPUT_ID, Some(&capability)).unwrap();

        assert_eq!(modes.len(), 3);
        assert_eq!(
            modes[0],
            DisplayMode::new(640, 480, 60_000, 32, PixelFormat::Xrgb)
        );
        assert_eq!(
            modes[2],
            DisplayMode::new(1024, 768, 60_000, 32, PixelFormat::Xrgb)
        );
    }

    #[test]
    fn mock_gpu_sets_supported_mode() {
        let mut service = service();
        let capability = display_capability();
        let mode = DisplayMode::new(800, 600, 60_000, 32, PixelFormat::Xrgb);

        assert_eq!(service.set_mode(OUTPUT_ID, mode, Some(&capability)), Ok(()));
        assert_eq!(service.device().active_mode(), mode);
    }

    #[test]
    fn mock_gpu_returns_framebuffer_for_active_mode() {
        let mut service = service();
        let capability = display_capability();
        let mode = DisplayMode::new(1024, 768, 60_000, 32, PixelFormat::Xrgb);
        service
            .set_mode(OUTPUT_ID, mode, Some(&capability))
            .unwrap();

        let framebuffer = service.framebuffer(OUTPUT_ID, Some(&capability)).unwrap();

        assert_eq!(framebuffer.mode(), mode.framebuffer_mode().unwrap());
        assert_eq!(framebuffer.memory().len(), 1024 * 768 * 4);
    }

    #[test]
    fn mock_gpu_denies_access_without_capability() {
        let mut service = service();
        let mode = DisplayMode::new(800, 600, 60_000, 32, PixelFormat::Xrgb);

        assert_eq!(service.modes(OUTPUT_ID, None), Err(GpuError::AccessDenied));
        assert_eq!(
            service.set_mode(OUTPUT_ID, mode, None),
            Err(GpuError::AccessDenied)
        );
        assert_eq!(
            service.framebuffer(OUTPUT_ID, None),
            Err(GpuError::AccessDenied)
        );
    }

    #[test]
    fn mock_gpu_denies_wrong_or_revoked_capability() {
        let mut wrong_output = GpuCapability::display(DEVICE_ID, 1);
        let service = service();

        assert_eq!(
            service.modes(OUTPUT_ID, Some(&wrong_output)),
            Err(GpuError::AccessDenied)
        );

        wrong_output = display_capability();
        wrong_output.revoke();
        assert_eq!(
            service.modes(OUTPUT_ID, Some(&wrong_output)),
            Err(GpuError::CapabilityRevoked)
        );
    }
}
