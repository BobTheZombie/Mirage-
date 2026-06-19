# i8042 / PS/2 keyboard bring-up

Mirage treats the i8042 controller as a lower-kernel mechanism and exposes only
facts and decoded input events upward.  The supervisor must not read or write the
`0x60/0x64` ports directly.

## Boot-safety rule

Keyboard input is never a boot dependency.  The driver may report:

- `PS/2 Keyboard [Started: polling mode]`
- `PS/2 Keyboard [Started: irq mode]`
- `PS/2 Keyboard [Failed: reason]`
- `Input [Skipped: reason]`

but post-kernel phases must continue to `boot info applied`, supervisor creation,
root mount, supervisor initialization, and MTSS online.  `Online` is only emitted
after a real decoded key event, never merely because initialization completed.

## Controller sequence

The i8042 path uses the standard ports:

- data: `0x60`
- status: `0x64`
- command: `0x64`

Initialization disables both PS/2 ports, flushes bounded pending output, reads
and rewrites the controller configuration with IRQs disabled during setup, tests
the controller and first port, intentionally disables translation when Set 2 is
preferred, then enables the first port.  IRQ1 is only enabled when the caller
requests IRQ mode; current early architecture bring-up uses polling mode so the
keyboard cannot interrupt the post-kernel boot pipeline.

All waits are bounded.  Timeouts and controller parity/timeout status bits return
typed errors instead of panicking.

## PS/2 command path

Keyboard commands (`reset`, `identify`, `disable scanning`, `set scancode set`,
`enable scanning`) are normal-thread setup transactions only.  The command helper
retries `RESEND` a bounded number of times and never runs from IRQ context.
During early boot these commands are best-effort: a VirtualBox/QEMU timing quirk
or missing response is logged honestly, but the driver still starts in degraded
polling mode so later scan bytes can be decoded.

## IRQ and polling

The IRQ1 handler performs only non-blocking data reads, decoder state updates,
fixed-queue publishing, and PIC/APIC EOI through the common interrupt dispatcher.
It never waits for ACK/BAT/RESEND, never allocates, never logs per scancode, and
uses `try_lock` so an interrupt cannot spin on a lock held by boot code.

Polling mode reads only already-available bytes and drains a bounded number per
call.  It is used before interrupts are a dependency and by the debug-shell
hotkey path.

## Scancode decoding

The driver supports translated Set 1 and native Set 2.  Native Set 2 handles
make/break (`0xf0`), extended (`0xe0`), and basic Pause-prefix poisoning
avoidance (`0xe1`).  Events include physical key code, press/release state,
modifiers, and optional US ASCII.

## Verbose debug

Default boot does not dump raw scancodes or redraw on every event.  Future command
line switches should use the reserved names `mirage.debug.keyboard=1` and
`mirage.debug.scancode=1` for opt-in verbose input diagnostics.
