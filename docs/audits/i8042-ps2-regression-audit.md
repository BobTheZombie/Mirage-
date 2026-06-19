# i8042 / PS/2 regression audit

## Root cause

The risky regression pattern was treating keyboard readiness as an early boot
synchronization point.  The old path could perform command/response waits during
bring-up and the IRQ path used blocking reads and blocking spin locks.  If IRQ1
arrived while boot or the debug path held the keyboard/input queue state, or if a
virtual PS/2 keyboard delayed ACK/BAT bytes, boot could stop after:

```text
PS/2 Keyboard [Started: irq mode]
kernel constructed
```

without reaching `boot info applied`.

## Fix summary

- i8042 reads used by IRQ context are non-blocking.
- PS/2 command transactions remain bounded and are never called from IRQ.
- Early architecture bring-up starts PS/2 in polling mode, keeping IRQ1 out of
  the critical post-kernel pipeline.
- Keyboard setup command failures are logged as honest degraded statuses but do
  not make boot wait for the first keypress.
- The input queue has a non-blocking IRQ producer path and drop accounting.
- `Online` is only set after a decoded key event.

## QEMU and VirtualBox notes

QEMU and VirtualBox differ in when the virtual keyboard returns BAT, identify,
and scancode-set responses.  Mirage therefore accepts partial setup and decodes
later scan bytes in degraded mode.  VirtualBox without an automated repo script
should be tested manually by attaching `build/mirage.iso` to a VM and verifying
that boot reaches `MTSS initialized` or `MTSS online` and does not stop after
`kernel constructed`.

## Regression check

`tools/check-boot-order.sh` runs QEMU under a timeout, captures boot output, and
fails if `kernel constructed` appears without `boot info applied` or if MTSS does
not initialize/come online.

## Remaining limitations

The production architecture should later move most input policy to supervised
`inputd` and add command-line controlled verbose scancode tracing.  The early
kernel path remains intentionally small and boot-safe.
