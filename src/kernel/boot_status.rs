//! No-heap boot milestone status for Mirage's persistent boot screen.
//!
//! The status model is a fixed-size structure so early x86_64 boot code can
//! update and render subsystem state before allocator, MTSS, or supervisor
//! policy is available.

/// Early boot state displayed by the persistent boot status screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootState {
    Pending,
    Ok,
    Online,
    Enabled,
    Failed,
    Skipped,
}

impl BootState {
    /// Stable text used by both framebuffer and serial renderers.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Ok => "OK",
            Self::Online => "Online",
            Self::Enabled => "Enabled",
            Self::Failed => "Failed",
            Self::Skipped => "Skipped",
        }
    }
}

/// Fixed boot status for Mirage Boot Milestone 1.0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootStatus {
    pub architecture: BootState,
    pub bootloader: BootState,
    pub framebuffer: BootState,
    pub idt: BootState,
    pub pic: BootState,
    pub interrupts: BootState,
    pub memory: BootState,
    pub paging: BootState,
    pub heap: BootState,
    pub mtss: BootState,
    pub supervisor: BootState,
    pub framebuffer_width: u64,
    pub framebuffer_height: u64,
    pub framebuffer_bpp: u16,
}

impl BootStatus {
    /// Construct an all-pending status with no framebuffer mode recorded.
    pub const fn new() -> Self {
        Self {
            architecture: BootState::Pending,
            bootloader: BootState::Pending,
            framebuffer: BootState::Pending,
            idt: BootState::Pending,
            pic: BootState::Pending,
            interrupts: BootState::Pending,
            memory: BootState::Pending,
            paging: BootState::Pending,
            heap: BootState::Pending,
            mtss: BootState::Pending,
            supervisor: BootState::Pending,
            framebuffer_width: 0,
            framebuffer_height: 0,
            framebuffer_bpp: 0,
        }
    }

    pub const fn record_framebuffer_mode(mut self, width: u64, height: u64, bpp: u16) -> Self {
        self.framebuffer_width = width;
        self.framebuffer_height = height;
        self.framebuffer_bpp = bpp;
        self
    }

    pub fn set_framebuffer_mode(&mut self, width: u64, height: u64, bpp: u16) {
        self.framebuffer_width = width;
        self.framebuffer_height = height;
        self.framebuffer_bpp = bpp;
    }
}

impl Default for BootStatus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{BootState, BootStatus};

    #[test]
    fn default_status_is_pending_without_framebuffer_mode() {
        let status = BootStatus::new();

        assert_eq!(status.architecture, BootState::Pending);
        assert_eq!(status.bootloader, BootState::Pending);
        assert_eq!(status.framebuffer, BootState::Pending);
        assert_eq!(status.idt, BootState::Pending);
        assert_eq!(status.pic, BootState::Pending);
        assert_eq!(status.interrupts, BootState::Pending);
        assert_eq!(status.memory, BootState::Pending);
        assert_eq!(status.paging, BootState::Pending);
        assert_eq!(status.heap, BootState::Pending);
        assert_eq!(status.mtss, BootState::Pending);
        assert_eq!(status.supervisor, BootState::Pending);
        assert_eq!(status.framebuffer_width, 0);
        assert_eq!(status.framebuffer_height, 0);
        assert_eq!(status.framebuffer_bpp, 0);
    }

    #[test]
    fn records_framebuffer_mode_without_allocation() {
        let mut status = BootStatus::new();

        status.set_framebuffer_mode(1024, 768, 32);

        assert_eq!(status.framebuffer_width, 1024);
        assert_eq!(status.framebuffer_height, 768);
        assert_eq!(status.framebuffer_bpp, 32);
    }

    #[test]
    fn state_labels_are_stable() {
        assert_eq!(BootState::Ok.as_str(), "OK");
        assert_eq!(BootState::Online.as_str(), "Online");
        assert_eq!(BootState::Enabled.as_str(), "Enabled");
        assert_eq!(BootState::Pending.as_str(), "Pending");
        assert_eq!(BootState::Failed.as_str(), "Failed");
        assert_eq!(BootState::Skipped.as_str(), "Skipped");
    }
}
