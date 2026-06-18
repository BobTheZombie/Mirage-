# i8042 / AT PS/2 Keyboard Input Path

Mirage treats the internal laptop keyboard as a lower-kernel hardware device: the x86_64 i8042 driver owns ports `0x60` and `0x64`, IRQ1, controller commands, bounded command waits, and raw scan-code reads. The supervisor owns policy: debug-shell routing, kernel-console routing, future userspace visibility, and recovery decisions.

## Controller initialization

The controller path disables PS/2 ports, flushes stale output bytes, reads the configuration byte, disables controller IRQ bits during setup, requests raw Set 2 by clearing translation when possible, optionally tests the controller and keyboard port, enables the first PS/2 port, and enables IRQ1 when IRQ mode is selected. Missing aux/mouse support is non-fatal.

Every hardware wait is bounded. Timeout, parity, self-test, port-test, device-response, and RESEND-exhaustion failures are represented as typed errors.

## Keyboard protocol

The AT keyboard driver sends reset (`0xff`), identify (`0xf2`), Set 2 selection (`0xf0 0x02`) when the controller is not translating, and enable scanning (`0xf4`). ACK (`0xfa`), RESEND (`0xfe`), and BAT OK (`0xaa`) are handled with bounded retries and timeouts.

## IRQ and polling

Polling mode is used during early boot and as fallback. IRQ mode installs IDT vector 33 and unmasks PIC IRQ1 only after the lower-kernel driver has initialized. The IRQ handler reads at most the available controller byte, ignores aux bytes, decodes or queues the event, and sends EOI through the existing interrupt-controller abstraction. It never runs command transactions.

## Structured events

Raw Set 2 bytes decode to `KeyboardEvent` with `KeyCode`, `KeyState`, modifiers, optional ASCII, raw source, and raw code. Events are published into a bounded no-heap queue with overflow accounting and can be consumed by the debug shell, kernel device reads, or a future Spider-rs/PID1 input service boundary.

## Status semantics

`Online` means real hardware scan bytes have decoded into a structured key event. Successful command initialization without a decoded event remains `Started`/`Ok`, not `Online`.

## Known limitations

The current layout is US keyboard only. Arrow-key history in the debug shell is pending. Userspace ABI export is defined as a registration point but not yet a full input service. Touchpad support is separate and is expected to arrive through ACPI/I2C-HID, not the i8042 keyboard path.
