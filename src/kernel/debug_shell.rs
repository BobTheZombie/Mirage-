//! Early kernel debug-shell stub entered from the boot idle loop.
//!
//! This is deliberately not a userspace shell. It has no filesystem, heap, or
//! supervisor-service dependency; it only preserves timer dispatch and CPU idle
//! behaviour while an early debug path is requested.

use crate::arch::x86_64;
use crate::kernel::boot_screen::render_persistent_boot_screen;
use crate::kernel::Kernel;

/// Enter the early debug-shell stub.
pub fn enter_early_debug_shell<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
) -> ! {
    crate::kprintln!("debug shell requested");
    crate::kprintln!("Mirage early debug shell");
    crate::kprintln!("commands: help, status, reboot(not implemented), halt");
    render_persistent_boot_screen();

    let mut observed_timer_ticks = x86_64::timer_ticks();
    loop {
        if x86_64::timer_tick_pending(&mut observed_timer_ticks) {
            kernel.tick();
        }
        x86_64::idle_halt();
    }
}
