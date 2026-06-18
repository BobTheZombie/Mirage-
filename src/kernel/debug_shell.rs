//! Early kernel debug-shell stub entered from the boot idle loop.
//!
//! This is deliberately not a userspace shell. It has no filesystem, heap, or
//! supervisor-service dependency; it only preserves timer dispatch and CPU idle
//! behaviour while an early debug path is requested.

use crate::arch::x86_64;
use crate::kernel::boot_screen::render_persistent_boot_screen;
use crate::kernel::input::{pop_keyboard_event, KeyCode, KeyState};
use crate::kernel::Kernel;

/// Enter the early debug-shell stub.
pub fn enter_early_debug_shell<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
) -> ! {
    crate::kprintln!("debug shell requested");
    crate::kprintln!("Mirage early debug shell");
    crate::kprintln!(
        "commands: help, status, input, keyboard, kbdstat, reboot(not implemented), halt"
    );
    render_persistent_boot_screen();

    let mut observed_timer_ticks = x86_64::timer_ticks();
    let mut line = [0u8; 128];
    let mut len = 0usize;
    crate::kprint!("mirage> ");
    loop {
        if x86_64::timer_tick_pending(&mut observed_timer_ticks) {
            kernel.tick();
        }
        #[cfg(feature = "hw-ps2-keyboard")]
        crate::arch::x86_64::ps2_keyboard::PS2_KEYBOARD_DRIVER.poll_hardware();
        while let Some(event) = pop_keyboard_event() {
            if event.state != KeyState::Pressed {
                continue;
            }
            match event.keycode {
                KeyCode::Enter => {
                    crate::kprintln!("");
                    run_command(&line[..len]);
                    len = 0;
                    crate::kprint!("mirage> ");
                }
                KeyCode::Backspace => {
                    if len > 0 {
                        len -= 1;
                        crate::kprint!("\x08 \x08");
                    }
                }
                _ => {
                    if let Some(byte) = event.ascii {
                        if byte >= 0x20 && byte < 0x7f && len < line.len() {
                            line[len] = byte;
                            len += 1;
                            crate::kprint!("{}", byte as char);
                        }
                    }
                }
            }
        }
        x86_64::idle_halt();
    }
}

fn run_command(command: &[u8]) {
    match command {
        b"help" => crate::kprintln!("commands: help, status, input, keyboard, kbdstat, halt"),
        b"status" => crate::kprintln!("debug shell active"),
        b"input" | b"keyboard" | b"kbdstat" => print_input_status(),
        b"halt" => crate::arch::x86_64::panic_halt(),
        b"" => {}
        _ => crate::kprintln!("unknown command"),
    }
}

fn print_input_status() {
    crate::kprintln!(
        "input queue depth={} overflows={}",
        crate::kernel::input::input_queue_depth(),
        crate::kernel::input::input_queue_overflows()
    );
    #[cfg(feature = "hw-ps2-keyboard")]
    {
        let snapshot = crate::arch::x86_64::ps2_keyboard::PS2_KEYBOARD_DRIVER.status_snapshot();
        crate::kprintln!(
            "ps2: initialized={} online={} mode={} scan_set={:?} events={} decode_errors={} overflows={}",
            snapshot.initialized,
            snapshot.online,
            if snapshot.irq_mode { "irq" } else { "polling" },
            snapshot.scan_set,
            snapshot.events_received,
            snapshot.decode_errors,
            snapshot.queue_overflows
        );
        if let Some(event) = snapshot.last_event {
            crate::kprintln!(
                "last key: {:?} {:?} ascii={:?} raw={:#x}",
                event.keycode,
                event.state,
                event.ascii,
                event.raw_code
            );
        }
    }
}
