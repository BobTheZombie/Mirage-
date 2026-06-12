//! Persistent no-heap boot screen renderer backed by the boot phase manager.

#[cfg(feature = "hw-framebuffer")]
use core::fmt::{self, Write};

#[cfg(feature = "hw-framebuffer")]
use crate::kernel::boot_phase::{boot_phase_progress_percent, BootPhase, PhaseState};
use crate::kernel::boot_phase::{boot_phase_snapshot, BootPhaseManager};
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
    framebuffer_console::write_colored("                    ", RgbColor::WHITE);
    framebuffer_console::write_colored(TITLE, RgbColor::CYAN);
    framebuffer_console::write_colored("\n\n", RgbColor::WHITE);
    framebuffer_console::write_colored("                ", RgbColor::WHITE);
    framebuffer_console::write_colored(SUBTITLE, RgbColor::WHITE);
    framebuffer_console::write_colored("\n\n", RgbColor::WHITE);

    fb_status(manager, "Seed-rs", BootPhase::SeedRs, None);
    fb_status(manager, "BootInfo", BootPhase::BootInfo, None);
    fb_status(manager, "Architecture", BootPhase::Architecture, None);
    fb_status(manager, "Serial", BootPhase::Serial, None);
    fb_status(manager, "GDT", BootPhase::Gdt, None);
    fb_status(manager, "Memory", BootPhase::Memory, None);
    fb_status(manager, "Paging", BootPhase::KernelMapper, None);
    fb_status(manager, "Heap", BootPhase::Heap, Some("ONLINE"));
    fb_status(
        manager,
        "Framebuffer",
        BootPhase::Framebuffer,
        Some("ONLINE"),
    );
    fb_status(manager, "IDT", BootPhase::Idt, None);
    fb_status(manager, "PIC", BootPhase::Pic, None);
    fb_status(
        manager,
        "Interrupts",
        BootPhase::Interrupts,
        Some("ENABLED"),
    );
    fb_status(manager, "Supervisor", BootPhase::Supervisor, None);
    fb_status(manager, "Root FS", BootPhase::RootFs, None);
    fb_status(manager, "Userspace", BootPhase::Userspace, None);
    fb_status(manager, "MTSS", BootPhase::Mtss, None);
    fb_status(manager, "Input", BootPhase::InputSubsystem, None);
    fb_status(manager, "PS/2 Kbd", BootPhase::Ps2Keyboard, None);
    fb_status(manager, "USB Kbd", BootPhase::UsbHidKeyboard, None);
    fb_status(manager, "EC Hotkeys", BootPhase::AcpiEcHotkeys, None);

    framebuffer_console::write_colored("\nBoot Progress\n", RgbColor::WHITE);
    fb_progress_bar(boot_phase_progress_percent(), manager.has_failed());
    framebuffer_console::write_colored("\n\nCurrent Phase:\n", RgbColor::WHITE);
    framebuffer_console::write_colored(manager.current_phase.name(), RgbColor::YELLOW);
    framebuffer_console::write_colored("\n\n", RgbColor::WHITE);
    framebuffer_console::write_colored(PROMPT, RgbColor::GRAY);
    framebuffer_console::write_colored("\n", RgbColor::WHITE);
}

#[cfg(feature = "hw-framebuffer")]
fn fb_status(
    manager: &BootPhaseManager,
    label: &str,
    phase: BootPhase,
    ok_alias: Option<&'static str>,
) {
    use crate::arch::x86_64::framebuffer_console::{self, RgbColor};

    let state = manager.state(phase);
    let value = display_value(state, ok_alias);
    write_label(label);
    framebuffer_console::write_colored(value, status_color(state));
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
fn display_value(state: PhaseState, ok_alias: Option<&'static str>) -> &'static str {
    match (state, ok_alias) {
        (PhaseState::Ok, Some(alias)) => alias,
        _ => state.as_str(),
    }
}

#[cfg(feature = "hw-framebuffer")]
fn status_color(state: PhaseState) -> crate::arch::x86_64::framebuffer_console::RgbColor {
    use crate::arch::x86_64::framebuffer_console::RgbColor;

    match state {
        PhaseState::Ok => RgbColor::GREEN,
        PhaseState::Pending | PhaseState::Started => RgbColor::YELLOW,
        PhaseState::Stub | PhaseState::Skipped => RgbColor::CYAN,
        PhaseState::Failed => RgbColor::RED,
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
