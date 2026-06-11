//! Persistent no-heap boot status renderer for Mirage Boot Milestone 1.0.

#[cfg(feature = "hw-framebuffer")]
use core::fmt::{self, Write};

use crate::kernel::boot_status::{BootState, BootStatus};

const TITLE: &str = "GNU/MIRAGE";
const SUBTITLE: &str = "Mirage kernel boot complete";
const PROMPT: &str = "Press ESC for debug shell";

/// Render the persistent framebuffer boot screen and mirror it to serial.
///
/// The framebuffer path clears exactly once for this render call and then draws
/// the full status screen.  The serial path is plain text and remains available
/// even when no framebuffer was initialized.
pub fn render_persistent_boot_screen(status: &BootStatus) {
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
    crate::kprintln!("");
    crate::kprintln!("{}", PROMPT);
}

#[cfg(feature = "hw-framebuffer")]
fn render_framebuffer(status: &BootStatus) {
    use crate::arch::x86_64::framebuffer_console::{self, RgbColor};

    framebuffer_console::clear_screen();
    framebuffer_console::write_colored(TITLE, RgbColor::CYAN);
    framebuffer_console::write_colored("\n\n", RgbColor::WHITE);
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
    fb_status("IDT", status.idt);
    fb_status("PIC", status.pic);
    fb_status("Interrupts", status.interrupts);
    fb_status("Memory", status.memory);
    fb_status("Paging", status.paging);
    fb_status("Heap", status.heap);
    fb_status("MTSS", status.mtss);
    fb_status("Supervisor", status.supervisor);

    framebuffer_console::write_colored("\n", RgbColor::WHITE);
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
    framebuffer_console::write_colored("\n", RgbColor::WHITE);
}

#[cfg(feature = "hw-framebuffer")]
fn write_label(label: &str) {
    use crate::arch::x86_64::framebuffer_console::{self, RgbColor};

    let mut writer = FramebufferColorWriter(RgbColor::GRAY);
    let _ = write!(writer, "{:<12} : ", label);
    framebuffer_console::write_colored("", RgbColor::WHITE);
}

#[cfg(feature = "hw-framebuffer")]
fn status_color(state: BootState) -> crate::arch::x86_64::framebuffer_console::RgbColor {
    use crate::arch::x86_64::framebuffer_console::RgbColor;

    match state {
        BootState::Ok | BootState::Online | BootState::Enabled => RgbColor::GREEN,
        BootState::Pending | BootState::Skipped => RgbColor::YELLOW,
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
