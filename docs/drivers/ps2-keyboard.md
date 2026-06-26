# PS/2 Keyboard Driver

The PS/2 keyboard is an optional boot input device. Mirage may boot without a keyboard.

## Polling mode

Polling mode means non-blocking `poll_keyboard_once` or bounded `drain_keyboard_events(max_events)`. It never means waiting forever for a key. A poll returns immediately when no data is available.

## Probe/start policy

The driver initializes the controller, enables the keyboard port, and treats device commands such as reset, BAT, identify, and set-scancode as best-effort. Failures are reported as degraded status and boot continues.

## Scancode decoding

The decoder handles translated set-1 make/break bytes and minimal set-2/extended prefixes. Unknown scancodes are represented as raw key codes and must not panic.

## IRQ versus polling

IRQ handlers must read at most bounded pending bytes, avoid blocking, and avoid heavy parsing. Until IRQ1 is fully validated, Mirage starts the PS/2 keyboard in polling mode.
