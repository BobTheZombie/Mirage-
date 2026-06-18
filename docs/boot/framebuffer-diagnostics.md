# Framebuffer Boot Diagnostics

The Mirage framebuffer boot UI is a live milestone screen, not a raw diagnostic
console. Its default job is to show concise boot progress without making boot
frame-by-frame slow.

## Default live milestone UI

When a hardware framebuffer is available, Mirage displays the boot phase table,
current phase, and weighted progress bar live. Phase/status changes render
immediately so users can see progress during normal QEMU, VirtualBox, and bare
metal boots.

Framebuffer renders are triggered by canonical phase status transitions, including
states displayed as:

- `Registered`
- `Started`
- `Detected`
- `Found`
- `Ok`
- `Online`
- `Skipped`
- `Stub`
- `Failed`
- `Pending`
- `Runnable`

Registration remains silent by default; the registered/pending rows appear as
part of the live table once boot starts advancing.

## What is not rendered by default

The framebuffer is not updated for every diagnostic breadcrumb. These events are
kept out of the default UI path:

- `boot_trace_substep` breadcrumbs;
- raw PCI/Ryzen/hardware field dumps;
- serial-only debug messages;
- repeated non-phase log lines.

Those events may be captured in the boot diagnostics ring or sent to serial when
verbose diagnostics are enabled, but they must not redraw the framebuffer in the
fast default mode.

## Clear/redraw policy

The boot UI may clear the framebuffer once when entering the milestone screen.
After that, phase refreshes rewind the text cursor and overwrite the fixed table
instead of clearing the whole screen repeatedly. Fatal failure diagnostics are the
exception: a failure screen may take over and persist so crash evidence remains
visible.

## Verbose framebuffer diagnostics

`bootdiag-framebuffer` is reserved for optional framebuffer diagnostic overlays or
log fanout and should be treated as intentionally slow. Prefer serial diagnostics
for raw hardware bring-up, and only enable verbose framebuffer diagnostics when
the framebuffer path itself is being debugged.
