# i8042 / PS/2 Keyboard Audit

Audited files included `src/arch/x86_64/mod.rs`, `src/arch/x86_64/interrupts.rs`, `src/arch/x86_64/idt.rs`, `src/arch/x86_64/pic.rs`, `src/arch/x86_64/irq.rs`, `src/arch/x86_64/i8042.rs`, `src/arch/x86_64/ps2_keyboard.rs`, `src/kernel/input.rs`, `src/kernel/debug_shell.rs`, `src/kernel/device.rs`, `src/supervisor/mod.rs`, USB HID keyboard code, and boot-phase registration.

## Findings

The repository already had a real but minimal i8042/PS2 path. It probed and initialized the controller, read scan codes in polling mode, had a partial Set 1/Set 2 decoder, and published keyboard events into a bounded kernel queue. IRQ1 had an IDT entry but the generic external IRQ path did not dispatch to the PS/2 driver. The debug-shell hotkey path polled for ESC but the shell itself did not consume typed input. The previous PS/2 path marked the keyboard source online immediately after command initialization, before a real decoded event.

## Implemented changes

The lower-kernel i8042 driver now exposes parsed status bits, typed controller errors, bounded wait helper entry points, status-state types, concise bring-up diagnostics, and documented narrow unsafe port I/O. The PS/2 keyboard path now differentiates Started from Online, records stats, supports IRQ1 dispatch, keeps polling fallback, and only marks online after a real decoded key event. The input layer carries additional keys/modifier bits and exposes queue diagnostics. The debug shell consumes structured events for line input and status commands. Supervisor input/i8042 policy modules record facts without port access.

## Remaining blockers

Bare-metal validation on the Dell Inspiron 15 5505 is still required to confirm firmware-specific controller behavior. Full alternate keymaps, LED state commands, typematic policy, APIC/IOAPIC IRQ routing, and a stable Spider-rs userspace input ABI remain future work.
