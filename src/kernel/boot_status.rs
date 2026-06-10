//! Fixed boot milestone tracking and stable early boot-complete output.
//!
//! This module intentionally avoids heap allocation.  The status structure is a
//! fixed set of booleans because it describes the architectural boot milestones
//! the early kernel wants to expose before supervisor-backed services exist.

/// Kernel boot milestone that can be marked complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootStage {
    Architecture,
    Memory,
    Supervisor,
    Mtss,
    IdleLoop,
}

/// Fixed boot status flags for the early kernel boot path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootStatus {
    architecture_initialized: bool,
    memory_initialized: bool,
    supervisor_initialized: bool,
    mtss_initialized: bool,
    idle_loop_entered: bool,
}

impl BootStatus {
    /// Create a boot status with no stages marked complete.
    pub const fn new() -> Self {
        Self {
            architecture_initialized: false,
            memory_initialized: false,
            supervisor_initialized: false,
            mtss_initialized: false,
            idle_loop_entered: false,
        }
    }

    /// Mark one boot stage as complete.
    pub fn mark(&mut self, stage: BootStage) {
        match stage {
            BootStage::Architecture => self.architecture_initialized = true,
            BootStage::Memory => self.memory_initialized = true,
            BootStage::Supervisor => self.supervisor_initialized = true,
            BootStage::Mtss => self.mtss_initialized = true,
            BootStage::IdleLoop => self.idle_loop_entered = true,
        }
    }

    pub const fn architecture_initialized(&self) -> bool {
        self.architecture_initialized
    }

    pub const fn memory_initialized(&self) -> bool {
        self.memory_initialized
    }

    pub const fn supervisor_initialized(&self) -> bool {
        self.supervisor_initialized
    }

    pub const fn mtss_initialized(&self) -> bool {
        self.mtss_initialized
    }

    pub const fn idle_loop_entered(&self) -> bool {
        self.idle_loop_entered
    }
}

impl Default for BootStatus {
    fn default() -> Self {
        Self::new()
    }
}

/// Print the stable boot-complete screen once for the early kernel console.
pub fn print_boot_complete_screen(status: &BootStatus) {
    emit_line("Mirage kernel boot complete");
    emit_line("Architecture: x86_64");
    emit_bool_line("Memory initialized: ", status.memory_initialized());
    emit_bool_line("MTSS initialized: ", status.mtss_initialized());
    emit_bool_line("Supervisor initialized: ", status.supervisor_initialized());
    emit_bool_line("Idle loop entered: ", status.idle_loop_entered());
    emit_line("");
    emit_line("Press ESC for debug shell...");
}

fn emit_bool_line(prefix: &'static str, value: bool) {
    if value {
        emit_parts(prefix, "yes");
    } else {
        emit_parts(prefix, "no");
    }
}

fn emit_line(line: &'static str) {
    crate::kprintln!("{}", line);
    framebuffer_write(line);
    framebuffer_write("\n");
}

fn emit_parts(prefix: &'static str, suffix: &'static str) {
    crate::kprintln!("{}{}", prefix, suffix);
    framebuffer_write(prefix);
    framebuffer_write(suffix);
    framebuffer_write("\n");
}

#[cfg(feature = "hw-framebuffer")]
fn framebuffer_write(text: &str) {
    crate::arch::x86_64::framebuffer_console::write_str(text);
}

#[cfg(not(feature = "hw-framebuffer"))]
fn framebuffer_write(_text: &str) {}

#[cfg(test)]
mod tests {
    use super::{BootStage, BootStatus};

    #[test]
    fn new_status_has_no_completed_stages() {
        let status = BootStatus::new();

        assert!(!status.architecture_initialized());
        assert!(!status.memory_initialized());
        assert!(!status.supervisor_initialized());
        assert!(!status.mtss_initialized());
        assert!(!status.idle_loop_entered());
    }

    #[test]
    fn marks_architecture_stage() {
        let mut status = BootStatus::new();
        status.mark(BootStage::Architecture);

        assert!(status.architecture_initialized());
        assert!(!status.memory_initialized());
        assert!(!status.supervisor_initialized());
        assert!(!status.mtss_initialized());
        assert!(!status.idle_loop_entered());
    }

    #[test]
    fn marks_memory_stage() {
        let mut status = BootStatus::new();
        status.mark(BootStage::Memory);

        assert!(!status.architecture_initialized());
        assert!(status.memory_initialized());
        assert!(!status.supervisor_initialized());
        assert!(!status.mtss_initialized());
        assert!(!status.idle_loop_entered());
    }

    #[test]
    fn marks_supervisor_stage() {
        let mut status = BootStatus::new();
        status.mark(BootStage::Supervisor);

        assert!(!status.architecture_initialized());
        assert!(!status.memory_initialized());
        assert!(status.supervisor_initialized());
        assert!(!status.mtss_initialized());
        assert!(!status.idle_loop_entered());
    }

    #[test]
    fn marks_mtss_stage() {
        let mut status = BootStatus::new();
        status.mark(BootStage::Mtss);

        assert!(!status.architecture_initialized());
        assert!(!status.memory_initialized());
        assert!(!status.supervisor_initialized());
        assert!(status.mtss_initialized());
        assert!(!status.idle_loop_entered());
    }

    #[test]
    fn marks_idle_loop_stage() {
        let mut status = BootStatus::new();
        status.mark(BootStage::IdleLoop);

        assert!(!status.architecture_initialized());
        assert!(!status.memory_initialized());
        assert!(!status.supervisor_initialized());
        assert!(!status.mtss_initialized());
        assert!(status.idle_loop_entered());
    }

    #[test]
    fn marks_all_stages_without_clearing_previous_stages() {
        let mut status = BootStatus::new();

        status.mark(BootStage::Architecture);
        status.mark(BootStage::Memory);
        status.mark(BootStage::Supervisor);
        status.mark(BootStage::Mtss);
        status.mark(BootStage::IdleLoop);

        assert!(status.architecture_initialized());
        assert!(status.memory_initialized());
        assert!(status.supervisor_initialized());
        assert!(status.mtss_initialized());
        assert!(status.idle_loop_entered());
    }
}
