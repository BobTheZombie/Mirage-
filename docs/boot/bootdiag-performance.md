# Boot Diagnostics Performance Defaults

Mirage keeps the normal boot path fast while still showing live visual progress.
The framebuffer boot milestone UI is not a verbose log sink: it is a phase table,
current-phase label, and progress bar that updates only when canonical boot phase
state changes.

## Default fast live boot

Default configuration:

```text
boot_screen_live = true when hw-framebuffer is available
phase_updates_live = true
substep_framebuffer = false
raw_hw_dump_framebuffer = false
serial_verbose = false
failure_screen = true
ring_capture = optional/minimal
```

In this mode:

- phase registration updates only the in-memory phase table and stays silent;
- meaningful phase/status transitions repaint the live milestone UI;
- the progress percentage/bar and current phase update as phases advance;
- major milestone state changes such as `Started`, `Detected`, `Found`, `Ok`,
  `Online`, `Skipped`, `Stub`, `Failed`, `Pending`, and `Runnable` are visible
  in the phase table when those states are reported;
- `boot_trace_substep` does not repaint the framebuffer by default;
- breadcrumbs and substeps are ring-buffer/serial-debug evidence, not live UI
  frames;
- raw PCI, Ryzen, ACPI, AHCI, NVMe, xHCI, framebuffer, keyboard, and userspace
  loader dumps are suppressed unless explicitly enabled;
- panic/fault/fatal paths still capture failure state and draw/report failure
  diagnostics once.

The boot UI clears the framebuffer once when entering the milestone screen. Later
phase refreshes rewind and overwrite the fixed table instead of repeatedly
clearing the whole display.

## Verbose diagnostic boot

Verbose diagnostics require explicit opt-in Cargo features:

```text
bootdiag              capture diagnostics ring entries
boot-trace           raw seed-rs/BootInfo COM1 breadcrumbs
bootdiag-verbose      verbose phase/substep diagnostics plus boot-trace
bootdiag-serial       serial diagnostic writes
bootdiag-framebuffer  optional framebuffer diagnostic overlays/log fanout
bootdiag-raw-hw       raw hardware dumps
```

Example:

```sh
cargo check --features 'bootdiag bootdiag-serial'
cargo check --features 'bootdiag-verbose bootdiag-framebuffer bootdiag-raw-hw hw-framebuffer'
```

`bootdiag-framebuffer` and especially `bootdiag-raw-hw` are intentionally slow on
many firmware framebuffers. They should not be used for normal boot timing or for
QEMU/VirtualBox/bare-metal performance checks unless the investigation is about
framebuffer diagnostics themselves.

## Recommended real-hardware debugging

Start with serial-only capture:

```text
bootdiag + bootdiag-serial
```

Only enable `bootdiag-raw-hw` when investigating a specific device discovery
failure. Avoid verbose framebuffer diagnostics on real hardware unless the
display path itself is under test. The default live milestone UI remains enabled
separately from verbose boot diagnostics.
