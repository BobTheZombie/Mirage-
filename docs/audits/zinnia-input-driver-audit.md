# Zinnia Input Driver Audit

## Zinnia commit inspected

`ecbedd86ab8fe70a5db02eabcf35966b77f0eb56`

## Zinnia license summary

Zinnia's repository contains a top-level `LICENSE` with GNU GPL version 2 text. Because Mirage must preserve its own provenance, this audit treated Zinnia as a reference only. No Zinnia code was copied into Mirage.

## Zinnia input/keyboard files inspected

- `kernel/src/arch/x86_64/system/ps2.rs`
- `kernel/src/device/input/mod.rs`
- `kernel/src/uapi/input.rs`
- USB/HID references found under `kernel/src/device/usb/`

## Mirage input/keyboard files inspected

- `src/arch/x86_64/i8042.rs`
- `src/arch/x86_64/ps2_keyboard.rs`
- `src/kernel/input.rs`
- `src/arch/x86_64/mod.rs`
- `src/arch/x86_64/idt.rs`
- `src/kernel/boot_phase.rs`

## What was learned

Zinnia keeps PS/2 hardware access in the architecture layer and converts keyboard bytes into input events rather than making higher layers read port `0x60` directly. Its PS/2 path showed useful separation between controller access, scancode decoding, and event-device delivery. It also made clear that IRQ handlers should be small and that input should be delivered through queues.

## What was reimplemented independently

Mirage now keeps polling mode as a bounded poll-once/bounded-drain path. The PS/2 driver reads data only after the i8042 output-buffer-full bit is set, ignores AUX bytes on the keyboard path, decodes incrementally, and pushes events into Mirage's bounded input queue. Boot reports PS/2 keyboard start and OK/degraded state without waiting forever for a user key.

## Whether any code was copied

No code was copied from Zinnia. The Mirage changes are independent Rust code using Mirage's existing driver, boot phase, and input queue structures.

## License/provenance note

Zinnia was used only for architectural study. Its GPL-2.0 license was inspected before implementation. Mirage source changes in this patch are Mirage-native and do not include copied Zinnia implementation text.

## Remaining gaps

- IRQ1 can later be promoted from polling mode after the interrupt route is fully validated.
- The PS/2 decoder is still intentionally minimal and not a full layout/evdev stack.
- Hardware reset/identify remains best-effort because early boot must not depend on a keyboard response.
