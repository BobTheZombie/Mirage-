//! Persistent no-heap boot status renderer for Mirage Boot Milestone 1.0.

#[cfg(feature = "hw-framebuffer")]
use core::fmt::{self, Write};

use crate::kernel::boot_status::{BootState, BootStatus};
use crate::kernel::sync::SpinLock;

const TITLE: &str = "GNU/MIRAGE";
const SUBTITLE: &str = "Mirage Boot Milestone 1.0";
const PROMPT: &str = "Press ESC for debug shell";
#[cfg(feature = "hw-framebuffer")]
const PROGRESS_BAR_WIDTH: u8 = 22;

static LAST_RENDERED_STATUS: SpinLock<Option<BootStatus>> = SpinLock::new(None);

/// Render the persistent framebuffer boot screen and mirror it to serial.
///
/// This routine is change-gated: repeated calls with an identical status are a
/// no-op so the idle loop can keep the screen persistent without spamming the
/// framebuffer or serial console.
pub fn render_persistent_boot_screen(status: &BootStatus) {
    {
        let mut last = LAST_RENDERED_STATUS.lock();
        if last.as_ref() == Some(status) {
            return;
        }
        *last = Some(*status);
    }

    render_serial(status);

    #[cfg(feature = "hw-framebuffer")]
    render_framebuffer(status);
}

fn render_serial(status: &BootStatus) {
    crate::kprintln!("{}", TITLE);
    crate::kprintln!("");
    crate::kprintln!("{}", SUBTITLE);
    crate::kprintln!("");
    crate::kprintln!("Architecture : {}", architecture_value(status));
    crate::kprintln!("Bootloader   : {}", bootloader_value(status));
    crate::kprintln!("Framebuffer  : {}", status.framebuffer.as_str());
    crate::kprintln!(
        "Resolution   : {}x{}x{}",
        status.framebuffer_width,
        status.framebuffer_height,
        status.framebuffer_bpp
    );
    crate::kprintln!("IDT          : {}", status.idt.as_str());
    crate::kprintln!("PIC          : {}", status.pic.as_str());
    crate::kprintln!("Interrupts   : {}", status.interrupts.as_str());
    crate::kprintln!("Memory       : {}", status.memory.as_str());
    crate::kprintln!("Paging       : {}", status.paging.as_str());
    crate::kprintln!("Heap         : {}", status.heap.as_str());
    crate::kprintln!("MTSS         : {}", status.mtss.as_str());
    crate::kprintln!("Supervisor   : {}", status.supervisor.as_str());
    crate::kprintln!("Root FS      : {}", status.root_fs.as_str());
    crate::kprintln!("Userspace    : {}", status.userspace.as_str());
    crate::kprintln!("");
    crate::kprintln!("Boot progress: {}%", status.boot_progress_percent());
    crate::kprintln!("Current stage: {}", status.current_stage_message());
    crate::kprintln!("");
    crate::kprintln!("{}", PROMPT);
}

#[cfg(feature = "hw-framebuffer")]
fn render_framebuffer(status: &BootStatus) {
    use crate::arch::x86_64::framebuffer_console::{self, RgbColor};

    framebuffer_console::clear_screen();
    framebuffer_console::write_colored("                    ", RgbColor::WHITE);
    framebuffer_console::write_colored(TITLE, RgbColor::CYAN);
    framebuffer_console::write_colored("\n\n", RgbColor::WHITE);
    framebuffer_console::write_colored("                ", RgbColor::WHITE);
    framebuffer_console::write_colored(SUBTITLE, RgbColor::WHITE);
    framebuffer_console::write_colored("\n\n", RgbColor::WHITE);

    fb_line_value(
        "Architecture",
        architecture_value(status),
        status.architecture,
    );
    fb_line_value("Bootloader", bootloader_value(status), status.bootloader);
    fb_status("Framebuffer", status.framebuffer);
    fb_resolution(status);
    framebuffer_console::write_colored("\n", RgbColor::WHITE);
    fb_status("IDT", status.idt);
    fb_status("PIC", status.pic);
    fb_status("Interrupts", status.interrupts);
    framebuffer_console::write_colored("\n", RgbColor::WHITE);
    fb_status("Memory", status.memory);
    fb_status("Paging", status.paging);
    fb_status("Heap", status.heap);
    framebuffer_console::write_colored("\n", RgbColor::WHITE);
    fb_status("MTSS", status.mtss);
    fb_status("Supervisor", status.supervisor);
    fb_status("Root FS", status.root_fs);
    fb_status("Userspace", status.userspace);

    framebuffer_console::write_colored("\nBoot Progress\n", RgbColor::WHITE);
    fb_progress_bar(status.boot_progress_percent());
    framebuffer_console::write_colored("\n\nCurrent Stage:\n", RgbColor::WHITE);
    framebuffer_console::write_colored(status.current_stage_message(), RgbColor::YELLOW);
    framebuffer_console::write_colored("\n\n", RgbColor::WHITE);
    framebuffer_console::write_colored(PROMPT, RgbColor::GRAY);
    framebuffer_console::write_colored("\n", RgbColor::WHITE);
}

#[cfg(feature = "hw-framebuffer")]
fn fb_status(label: &str, state: BootState) {
    crate::arch::x86_64::framebuffer_console::write_status(label, state.as_str(), state);
}

#[cfg(feature = "hw-framebuffer")]
fn fb_line_value(label: &str, value: &str, state: BootState) {
    use crate::arch::x86_64::framebuffer_console::{self, RgbColor};

    write_label(label);
    framebuffer_console::write_colored(value, status_color(state));
    framebuffer_console::write_colored(" ]", RgbColor::GRAY);
    framebuffer_console::write_colored("\n", RgbColor::WHITE);
}

#[cfg(feature = "hw-framebuffer")]
fn fb_resolution(status: &BootStatus) {
    use crate::arch::x86_64::framebuffer_console::{self, RgbColor};

    write_label("Resolution");
    let mut writer = FramebufferColorWriter(RgbColor::GREEN);
    let _ = write!(
        writer,
        "{}x{}x{}",
        status.framebuffer_width, status.framebuffer_height, status.framebuffer_bpp
    );
    framebuffer_console::write_colored(" ]", RgbColor::GRAY);
    framebuffer_console::write_colored("\n", RgbColor::WHITE);
}

#[cfg(feature = "hw-framebuffer")]
fn fb_progress_bar(percent: u8) {
    use crate::arch::x86_64::framebuffer_console::{self, RgbColor};

    let filled = ((percent as u16 * PROGRESS_BAR_WIDTH as u16) / 100) as u8;
    framebuffer_console::write_colored("[", RgbColor::GRAY);
    let mut index = 0u8;
    while index < PROGRESS_BAR_WIDTH {
        if index < filled {
            framebuffer_console::write_colored("#", RgbColor::GREEN);
        } else {
            framebuffer_console::write_colored("-", RgbColor::GRAY);
        }
        index += 1;
    }
    framebuffer_console::write_colored("] ", RgbColor::GRAY);
    let color = if percent == 100 {
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
fn status_color(state: BootState) -> crate::arch::x86_64::framebuffer_console::RgbColor {
    use crate::arch::x86_64::framebuffer_console::RgbColor;

    match state {
        BootState::Ok | BootState::Online | BootState::Enabled => RgbColor::GREEN,
        BootState::Pending => RgbColor::YELLOW,
        BootState::Stub | BootState::Skipped => RgbColor::CYAN,
        BootState::Failed => RgbColor::RED,
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

fn architecture_value(status: &BootStatus) -> &'static str {
    if status.architecture == BootState::Ok {
        "x86_64"
    } else {
        status.architecture.as_str()
    }
}

fn bootloader_value(status: &BootStatus) -> &'static str {
    if status.bootloader == BootState::Ok {
        "Limine"
    } else {
        status.bootloader.as_str()
    }
}
