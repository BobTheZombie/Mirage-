//! No-heap boot milestone status for Mirage's persistent boot screen.
//!
//! The status model is a fixed-size structure so early x86_64 boot code can
//! update and render subsystem state before allocator, MTSS, or supervisor
//! policy is available.

/// Early boot state displayed by the persistent boot status screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootState {
    Registered,
    Pending,
    Started,
    Detected,
    Ok,
    Online,
    Enabled,
    Stub,
    Skipped,
    Failed,
}

impl BootState {
    /// Stable text used by both framebuffer and serial renderers.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Registered => "Registered",
            Self::Pending => "Pending",
            Self::Started => "Started",
            Self::Detected => "Detected",
            Self::Ok => "Ok",
            Self::Online => "Online",
            Self::Enabled => "Enabled",
            Self::Stub => "Stub",
            Self::Skipped => "Skipped",
            Self::Failed => "Failed",
        }
    }

    /// Weighted progress numerator contribution for one component.
    ///
    /// Completed/online/enabled and explicitly skipped components count as
    /// complete.  Stubbed components count as half their assigned weight so the
    /// screen can distinguish deliberate milestone stubs from fully-online
    /// services without requiring heap-backed policy state.
    pub const fn progress_units(self, weight: u16) -> u16 {
        match self {
            Self::Ok | Self::Online | Self::Enabled | Self::Skipped => weight,
            Self::Detected | Self::Stub => weight / 2,
            Self::Started => (weight + 1) / 2,
            Self::Registered | Self::Pending | Self::Failed => 0,
        }
    }
}

/// Coarse boot stage used for the current-stage boot-screen message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootStage {
    SeedRs,
    BootInfo,
    Architecture,
    Framebuffer,
    Idt,
    Pic,
    Interrupts,
    Memory,
    Paging,
    Heap,
    Supervisor,
    RootFs,
    Mtss,
    Userspace,
    IdleLoop,
    Complete,
}

impl BootStage {
    /// Fixed, no-allocation display message for the current boot stage.
    pub const fn message(self) -> &'static str {
        match self {
            Self::SeedRs => "Entering SeedRs handoff",
            Self::BootInfo => "Reading Limine boot information",
            Self::Architecture => "Initializing x86_64 architecture",
            Self::Framebuffer => "Starting framebuffer console",
            Self::Idt => "Installing interrupt descriptor table",
            Self::Pic => "Programming interrupt controller",
            Self::Interrupts => "Enabling interrupts",
            Self::Memory => "Waiting for Memory Manager",
            Self::Paging => "Waiting for Paging Manager",
            Self::Heap => "Waiting for Heap Allocator",
            Self::Supervisor => "Starting Mirage Supervisor",
            Self::RootFs => "Mounting QFS root filesystem",
            Self::Mtss => "Initializing MTSS",
            Self::Userspace => "Preparing GNU/POSIX userspace stub",
            Self::IdleLoop => "Entering kernel idle loop",
            Self::Complete => "Boot complete",
        }
    }
}

const ARCHITECTURE_WEIGHT: u16 = 5;
const BOOTLOADER_WEIGHT: u16 = 5;
const FRAMEBUFFER_WEIGHT: u16 = 6;
const IDT_WEIGHT: u16 = 5;
const PIC_WEIGHT: u16 = 5;
const INTERRUPTS_WEIGHT: u16 = 5;
const MEMORY_WEIGHT: u16 = 13;
const PAGING_WEIGHT: u16 = 13;
const HEAP_WEIGHT: u16 = 13;
const MTSS_WEIGHT: u16 = 8;
const SUPERVISOR_WEIGHT: u16 = 8;
const ROOT_FS_WEIGHT: u16 = 8;
const USERSPACE_WEIGHT: u16 = 6;
const BOOT_PROGRESS_TOTAL_WEIGHT: u16 = ARCHITECTURE_WEIGHT
    + BOOTLOADER_WEIGHT
    + FRAMEBUFFER_WEIGHT
    + IDT_WEIGHT
    + PIC_WEIGHT
    + INTERRUPTS_WEIGHT
    + MEMORY_WEIGHT
    + PAGING_WEIGHT
    + HEAP_WEIGHT
    + MTSS_WEIGHT
    + SUPERVISOR_WEIGHT
    + ROOT_FS_WEIGHT
    + USERSPACE_WEIGHT;

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
    pub root_fs: BootState,
    pub userspace: BootState,
    pub current_stage: BootStage,
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
            root_fs: BootState::Pending,
            userspace: BootState::Pending,
            current_stage: BootStage::SeedRs,
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

    pub fn set_stage(&mut self, stage: BootStage) {
        self.current_stage = stage;
    }

    pub const fn current_stage_message(&self) -> &'static str {
        self.current_stage.message()
    }

    /// Compute the live weighted boot progress percentage.
    pub const fn boot_progress_percent(&self) -> u8 {
        let completed = self.progress_units();
        ((completed as u32 * 100) / BOOT_PROGRESS_TOTAL_WEIGHT as u32) as u8
    }

    pub const fn progress_units(&self) -> u16 {
        self.architecture.progress_units(ARCHITECTURE_WEIGHT)
            + self.bootloader.progress_units(BOOTLOADER_WEIGHT)
            + self.framebuffer.progress_units(FRAMEBUFFER_WEIGHT)
            + self.idt.progress_units(IDT_WEIGHT)
            + self.pic.progress_units(PIC_WEIGHT)
            + self.interrupts.progress_units(INTERRUPTS_WEIGHT)
            + self.memory.progress_units(MEMORY_WEIGHT)
            + self.paging.progress_units(PAGING_WEIGHT)
            + self.heap.progress_units(HEAP_WEIGHT)
            + self.mtss.progress_units(MTSS_WEIGHT)
            + self.supervisor.progress_units(SUPERVISOR_WEIGHT)
            + self.root_fs.progress_units(ROOT_FS_WEIGHT)
            + self.userspace.progress_units(USERSPACE_WEIGHT)
    }

    pub const fn progress_total_units(&self) -> u16 {
        BOOT_PROGRESS_TOTAL_WEIGHT
    }
}

impl Default for BootStatus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{BootStage, BootState, BootStatus};

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
        assert_eq!(status.root_fs, BootState::Pending);
        assert_eq!(status.userspace, BootState::Pending);
        assert_eq!(status.current_stage, BootStage::SeedRs);
        assert_eq!(status.framebuffer_width, 0);
        assert_eq!(status.framebuffer_height, 0);
        assert_eq!(status.framebuffer_bpp, 0);
        assert_eq!(status.boot_progress_percent(), 0);
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
        assert_eq!(BootState::Registered.as_str(), "Registered");
        assert_eq!(BootState::Started.as_str(), "Started");
        assert_eq!(BootState::Detected.as_str(), "Detected");
        assert_eq!(BootState::Ok.as_str(), "Ok");
        assert_eq!(BootState::Online.as_str(), "Online");
        assert_eq!(BootState::Enabled.as_str(), "Enabled");
        assert_eq!(BootState::Pending.as_str(), "Pending");
        assert_eq!(BootState::Failed.as_str(), "Failed");
        assert_eq!(BootState::Skipped.as_str(), "Skipped");
        assert_eq!(BootState::Stub.as_str(), "Stub");
    }

    #[test]
    fn weighted_progress_counts_stub_partially() {
        let mut status = BootStatus::new();
        status.architecture = BootState::Ok;
        status.bootloader = BootState::Ok;
        status.framebuffer = BootState::Online;
        status.idt = BootState::Ok;
        status.pic = BootState::Ok;
        status.interrupts = BootState::Enabled;
        status.mtss = BootState::Ok;
        status.supervisor = BootState::Ok;
        status.root_fs = BootState::Ok;
        status.userspace = BootState::Stub;

        assert_eq!(status.boot_progress_percent(), 58);
    }
}
