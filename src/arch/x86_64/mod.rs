//! Minimal 64-bit x86 support layer.
//!
//! The routines in this module intentionally avoid touching real hardware. They provide
//! structural placeholders that outline how a Rust kernel would prepare the processor
//! before handing over control to the higher level subsystems.

use core::hint::spin_loop;
use core::sync::atomic::{AtomicBool, Ordering};

pub mod clock;

pub use clock::{HardwareClock, HARDWARE_CLOCK};

static INITIALISED: AtomicBool = AtomicBool::new(false);

/// Perform one-time CPU and memory initialisation.
///
/// In a full kernel this would configure control registers, descriptor tables, and paging.
pub fn init_architecture() {
    if INITIALISED.swap(true, Ordering::SeqCst) {
        return;
    }

    configure_cpu_modes();
    setup_memory_layout();
    configure_interrupts();
}

/// Hint to the CPU that the current core is in a spin loop.
#[inline(always)]
pub fn cpu_relax() {
    spin_loop();
}

/// Halt the CPU in a panic scenario. In a real system an IPI or watchdog would reset us.
pub fn panic_halt() -> ! {
    loop {
        cpu_relax();
    }
}

fn configure_cpu_modes() {
    // Placeholder for enabling long mode, installing the GDT/IDT, and switching privilege levels.
}

fn setup_memory_layout() {
    // A real kernel would enable paging here and identity-map the critical regions. For Mirage we
    // simply model the presence of such logic to keep the example self-contained.
}

fn configure_interrupts() {
    // In the conceptual design we defer actual interrupt controller programming, but the hook is
    // left in place so higher layers can request timer ticks or I/O notifications later.
}
