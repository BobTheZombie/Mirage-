# Boot Diagnostics Slowdown Audit

## Cause

The boot diagnostics patch placed expensive work in the global boot hot path:
phase registration printed `[Phase] ... Registered` for every registered phase,
phase transitions wrote serial lines, and framebuffer rendering could redraw the
persistent boot screen after registration and state changes. Substep tracing also
locked diagnostics state, copied strings into the ring, formatted serial output,
and emitted entries for hardware discovery paths.

Serial output and firmware/framebuffer writes are both slow enough to make QEMU,
VirtualBox, and bare metal appear to boot frame-by-frame. Because the overhead
was attached to phase and substep APIs, it affected all boot paths, not only the
framebuffer path.

## Hot paths fixed

- Phase registration is now table-only by default.
- Phase transitions are state-only by default, except failures may still report.
- `boot_trace_substep` is off by default and returns before locks, copies, ring
  writes, serial output, or framebuffer work.
- Framebuffer live rendering requires `bootdiag-framebuffer`.
- Serial phase/substep logging requires `bootdiag-serial` or `bootdiag-verbose`.
- Raw hardware dumps require `bootdiag-raw-hw` or targeted environment debug
  overrides already used by the platform code.

## Classification

| Area | Default classification |
| --- | --- |
| Phase table registration | always-on and cheap |
| Phase status state changes | always-on and cheap |
| Failed phase reporting | must remain always-on and concise |
| Substep tracing | debug-only / capture-only when `bootdiag` is enabled |
| Serial verbose logs | serial-verbose only |
| Framebuffer live UI redraws | framebuffer-verbose only |
| Raw PCI/Ryzen/hardware dumps | raw-hardware debug only |
| Panic/fault failure screen | always available failure diagnostic |

## Performance counters

Boot diagnostics now expose cheap counters for captured events, ignored events,
framebuffer renders, serial writes, raw dumps suppressed, and dropped ring
entries. These counters are intended for debug-shell or final-summary display
when diagnostics are explicitly requested.

## Remaining risks

- Any future diagnostic call that formats strings before checking feature gates
  can reintroduce boot latency.
- Raw hardware debug features are intentionally noisy and slow.
- Framebuffer live rendering is useful for visual bring-up but should never be
  part of normal boot timing.
