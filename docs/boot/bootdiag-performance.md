# Boot Diagnostics Performance Defaults

Mirage boot diagnostics are intentionally off the global hot path by default.
Normal builds must boot quickly on QEMU, VirtualBox, and bare metal without
serial spam, framebuffer log scrolling, raw hardware dumps, or per-substep
rendering.

## Default fast boot

Default configuration:

```text
bootdiag = off
fb_live_log = false
serial_verbose = false
raw_hw_dump = false
substep_trace = false
failure_screen = true
```

In this mode:

- phase registration updates only the in-memory phase table;
- phase transitions update milestone state but do not print every transition;
- `boot_trace_substep` returns immediately and records an ignored-event counter;
- framebuffer boot-screen redraws are not performed for every event;
- raw PCI, Ryzen, ACPI, AHCI, NVMe, xHCI, framebuffer, keyboard, and userspace
  loader dumps are suppressed unless explicitly enabled;
- panic/fault/fatal paths still capture failure state and draw/report failure
  diagnostics once.

## Verbose diagnostic boot

Verbose diagnostics require explicit opt-in Cargo features:

```text
bootdiag              capture diagnostics ring entries
bootdiag-verbose      verbose phase/substep diagnostics
bootdiag-serial       serial diagnostic writes
bootdiag-framebuffer  live framebuffer boot UI/log rendering
bootdiag-raw-hw       raw hardware dumps
```

Example:

```sh
cargo check --features 'bootdiag bootdiag-serial'
cargo check --features 'bootdiag-verbose bootdiag-framebuffer bootdiag-raw-hw hw-framebuffer'
```

`bootdiag-framebuffer` is intentionally slow on many firmware framebuffers and
should not be used for normal boot timing. `bootdiag-raw-hw` can produce very
large serial output and should be reserved for targeted hardware bring-up.

## Recommended real-hardware debugging

Start with serial-only capture:

```text
bootdiag + bootdiag-serial
```

Only enable `bootdiag-raw-hw` when investigating a specific device discovery
failure. Avoid live framebuffer diagnostics on real hardware unless the display
path itself is under test.
