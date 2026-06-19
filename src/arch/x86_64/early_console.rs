//! x86_64 early console fanout.
//!
//! Serial remains the primary early diagnostic mechanism.  When the Limine
//! framebuffer console is available, this module mirrors formatted output there
//! on a best-effort basis without allocating or routing framebuffer errors back
//! through the kernel logging macros.

use core::fmt;
#[cfg(feature = "hw-framebuffer")]
use core::fmt::Write;

use crate::arch::x86_64::uart16550;

/// Write formatted text to the x86_64 early console path.
///
/// COM1 serial is always written first and remains authoritative.  The optional
/// framebuffer mirror is best-effort and deliberately ignores formatting/MMIO
/// failures so it can never recurse through `kprint!` or `kprintln!`.
pub fn write_fmt(args: fmt::Arguments<'_>) {
    uart16550::early_print(args);

    #[cfg(feature = "hw-framebuffer")]
    {
        if crate::kernel::boot_diagnostics::DEFAULT_FB_LOG_OVERLAY
            || crate::arch::x86_64::framebuffer_console::framebuffer_log_overlay_enabled()
        {
            let mut framebuffer = FramebufferMirror;
            let _ = framebuffer.write_fmt(args);
        }
    }
}

/// Panic-safe best-effort early console output.
///
/// This keeps the panic path serial-first, then mirrors to the framebuffer only
/// if the framebuffer lock can be taken immediately.  That avoids deadlocking on
/// a panic that interrupts framebuffer rendering while still giving visible
/// diagnostics when the framebuffer is idle.
pub fn panic_write_fmt(args: fmt::Arguments<'_>) {
    uart16550::panic_print(args);

    #[cfg(feature = "hw-framebuffer")]
    {
        let mut framebuffer = PanicFramebufferMirror;
        let _ = framebuffer.write_fmt(args);
    }
}

#[cfg(feature = "hw-framebuffer")]
struct FramebufferMirror;

#[cfg(feature = "hw-framebuffer")]
impl Write for FramebufferMirror {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        crate::arch::x86_64::framebuffer_console::write_str(text);
        Ok(())
    }
}

#[cfg(feature = "hw-framebuffer")]
struct PanicFramebufferMirror;

#[cfg(feature = "hw-framebuffer")]
impl Write for PanicFramebufferMirror {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        crate::arch::x86_64::framebuffer_console::try_write_str(text);
        Ok(())
    }
}
