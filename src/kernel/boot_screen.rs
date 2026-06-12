//! Persistent no-heap boot screen renderer backed by the boot phase manager.

#[cfg(feature = "hw-framebuffer")]
use core::fmt::{self, Write};

use crate::kernel::boot_phase::{boot_phase_snapshot, BootPhaseManager};
#[cfg(feature = "hw-framebuffer")]
use crate::kernel::boot_phase::{BootPhase, BootPhaseRecord, PhaseState};
use crate::kernel::sync::SpinLock;

#[cfg(feature = "hw-framebuffer")]
const TITLE: &str = "GNU/MIRAGE";
#[cfg(feature = "hw-framebuffer")]
const SUBTITLE: &str = "Mirage Boot Milestone 1.1";
#[cfg(feature = "hw-framebuffer")]
const PROMPT: &str = "Press ESC for debug shell";
#[cfg(feature = "hw-framebuffer")]
const PROGRESS_BAR_WIDTH: u8 = 28;

static LAST_RENDERED_PHASES: SpinLock<Option<BootPhaseManager>> = SpinLock::new(None);

/// Render the persistent framebuffer boot screen from the global boot phase state.
///
/// The renderer is change-gated and framebuffer-only. Serial diagnostics for
/// individual transitions are emitted directly by the boot phase manager, so
/// this screen refresh cannot spam COM1 and remains optional when no framebuffer
/// is present.
pub fn render_persistent_boot_screen() {
    let snapshot = boot_phase_snapshot();
    {
        let mut last = LAST_RENDERED_PHASES.lock();
        if last.as_ref() == Some(&snapshot) {
            return;
        }
        *last = Some(snapshot);
    }

    #[cfg(feature = "hw-framebuffer")]
    render_framebuffer(&snapshot);
}

#[cfg(feature = "hw-framebuffer")]
fn render_framebuffer(manager: &BootPhaseManager) {
    use crate::arch::x86_64::framebuffer_console::{self, RgbColor};

    framebuffer_console::clear_screen();
    framebuffer_console::write_colored(TITLE, RgbColor::CYAN);
    framebuffer_console::write_colored("\n\n", RgbColor::WHITE);
    framebuffer_console::write_colored("               ", RgbColor::WHITE);
    framebuffer_console::write_colored(SUBTITLE, RgbColor::WHITE);
    framebuffer_console::write_colored("\n\n", RgbColor::WHITE);

    render_group(manager, &CORE_SCREEN_PHASES);
    framebuffer_console::write_colored("\n", RgbColor::WHITE);
    render_group(manager, &SERVICE_SCREEN_PHASES);
    framebuffer_console::write_colored("\n", RgbColor::WHITE);
    render_group(manager, &INPUT_SCREEN_PHASES);

    framebuffer_console::write_colored("\nBoot Progress\n", RgbColor::WHITE);
    fb_progress_bar(manager.progress_percent(), manager.has_failed());
    framebuffer_console::write_colored("\n\nCurrent Phase:\n", RgbColor::WHITE);
    framebuffer_console::write_colored(manager.current_phase.friendly_name(), RgbColor::YELLOW);
    framebuffer_console::write_colored("\n\n", RgbColor::WHITE);
    framebuffer_console::write_colored(PROMPT, RgbColor::GRAY);
    framebuffer_console::write_colored("\n", RgbColor::WHITE);
}

#[cfg(feature = "hw-framebuffer")]
const CORE_SCREEN_PHASES: [BootPhase; 12] = [
    BootPhase::SeedRs,
    BootPhase::BootInfo,
    BootPhase::Architecture,
    BootPhase::Serial,
    BootPhase::Gdt,
    BootPhase::Memory,
    BootPhase::Paging,
    BootPhase::Heap,
    BootPhase::Framebuffer,
    BootPhase::Idt,
    BootPhase::Pic,
    BootPhase::Interrupts,
];

#[cfg(feature = "hw-framebuffer")]
const SERVICE_SCREEN_PHASES: [BootPhase; 4] = [
    BootPhase::Supervisor,
    BootPhase::RootFs,
    BootPhase::Userspace,
    BootPhase::Mtss,
];

#[cfg(feature = "hw-framebuffer")]
const INPUT_SCREEN_PHASES: [BootPhase; 6] = [
    BootPhase::Input,
    BootPhase::Xhci,
    BootPhase::UsbCore,
    BootPhase::UsbHid,
    BootPhase::UsbKeyboard,
    BootPhase::Ps2Keyboard,
    BootPhase::EcHotkeys,
];

#[cfg(feature = "hw-framebuffer")]
fn render_group(manager: &BootPhaseManager, phases: &[BootPhase]) {
    let mut index = 0usize;
    while index < phases.len() {
        if let Some(record) = manager.record(phases[index]) {
            fb_status(record);
        }
        index += 1;
    }
}

#[cfg(feature = "hw-framebuffer")]
fn fb_status(record: &BootPhaseRecord) {
    use crate::arch::x86_64::framebuffer_console::{self, RgbColor};

    write_label(record.descriptor.name);
    framebuffer_console::write_colored(record.state.as_str(), status_color(record.state));
    framebuffer_console::write_colored(" ]\n", RgbColor::GRAY);
}

#[cfg(feature = "hw-framebuffer")]
fn fb_progress_bar(percent: u8, failed: bool) {
    use crate::arch::x86_64::framebuffer_console::{self, RgbColor};

    let filled = ((percent as u16 * PROGRESS_BAR_WIDTH as u16) / 100) as u8;
    let fill_color = if failed {
        RgbColor::RED
    } else {
        RgbColor::GREEN
    };
    framebuffer_console::write_colored("[", RgbColor::GRAY);
    let mut index = 0u8;
    while index < PROGRESS_BAR_WIDTH {
        if index < filled {
            framebuffer_console::write_colored("#", fill_color);
        } else {
            framebuffer_console::write_colored("-", RgbColor::GRAY);
        }
        index += 1;
    }
    framebuffer_console::write_colored("] ", RgbColor::GRAY);
    let color = if failed {
        RgbColor::RED
    } else if percent == 100 {
        RgbColor::GREEN
    } else {
        RgbColor::YELLOW
    };
    let mut writer = FramebufferColorWriter(color);
    let _ = write!(writer, "{}%", percent);
}

#[cfg(feature = "hw-framebuffer")]
fn write_label(label: &str) {
    let mut writer =
        FramebufferColorWriter(crate::arch::x86_64::framebuffer_console::RgbColor::GRAY);
    let _ = write!(writer, "{:<12} [ ", label);
}

#[cfg(feature = "hw-framebuffer")]
fn status_color(state: PhaseState) -> crate::arch::x86_64::framebuffer_console::RgbColor {
    use crate::arch::x86_64::framebuffer_console::RgbColor;

    match state {
        PhaseState::Ok | PhaseState::Online | PhaseState::Enabled => RgbColor::GREEN,
        PhaseState::Started | PhaseState::Pending => RgbColor::YELLOW,
        PhaseState::Registered | PhaseState::Skipped => RgbColor::GRAY,
        PhaseState::Stub => RgbColor::CYAN,
        PhaseState::Failed => RgbColor::RED,
        PhaseState::Unregistered => RgbColor::GRAY,
    }
}

#[cfg(feature = "hw-framebuffer")]
struct FramebufferColorWriter(crate::arch::x86_64::framebuffer_console::RgbColor);

#[cfg(feature = "hw-framebuffer")]
impl Write for FramebufferColorWriter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        crate::arch::x86_64::framebuffer_console::write_colored(text, self.0);
        Ok(())
    }
}
