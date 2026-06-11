# Mirage Boot Milestone 1.0 — Persistent Boot Status Screen

## Milestone name

**Mirage Boot Milestone 1.0 — Persistent Boot Status Screen with Live Progress**

This milestone makes the first post-framebuffer boot state visible as an official
GNU/Mirage screen instead of relying only on scrolling early diagnostic logs. The
screen is persistent and now includes a weighted progress bar plus a fixed,
no-allocation current-stage message.

## Screenshot target

The QEMU framebuffer should remain on a black screen with aligned white/gray
labels, colored status words, a progress bar, and the current stage:

```text
                     GNU/MIRAGE

                 Mirage Boot Milestone 1.0

Architecture [ x86_64 ]
Bootloader   [ Limine ]
Framebuffer  [ ONLINE ]
Resolution   [ 1024x768x32 ]

IDT          [ OK ]
PIC          [ OK ]
Interrupts   [ ENABLED ]

Memory       [ PENDING ]
Paging       [ PENDING ]
Heap         [ PENDING ]

MTSS         [ OK ]
Supervisor   [ OK ]
Root FS      [ OK ]
Userspace    [ STUB ]

Boot Progress
[############----------] 58%

Current Stage:
Waiting for Memory Manager

Press ESC for debug shell
```

Actual resolution is supplied by Limine and may differ if QEMU is configured for
a different mode. The framebuffer font is an early ASCII bitmap font, so the bar
uses `#` and `-` instead of Unicode block glyphs.

## Required statuses

| Subsystem | Required display | Color |
| --- | --- | --- |
| Architecture | `x86_64` once x86_64 boot setup is active | green status color |
| Bootloader | `Limine` after the Limine handoff is accepted | green status color |
| Framebuffer | `ONLINE` when the Limine framebuffer console initializes | green |
| Resolution | `<width>x<height>x<bpp>` from framebuffer metadata | green |
| IDT | `OK` after `idt::initialize()` | green |
| PIC | `OK` after `pic::initialize()` | green |
| Interrupts | `ENABLED` after `interrupts::enable()` | green |
| Memory | `PENDING` for this milestone | yellow |
| Paging | `PENDING` for this milestone | yellow |
| Heap | `PENDING` for this milestone | yellow |
| MTSS | `OK` after `kernel.kernel_mtss_init()` | green |
| Supervisor | `OK` after the supervisor boot manifest succeeds | green |
| Root FS | `OK` after the QFS root mount path succeeds, including the built-in block QFS fallback | green |
| Userspace | `STUB` when the milestone intentionally skips real userspace | cyan |

`FAILED` is reserved for fault reporting and renders red when used. `SKIPPED`
represents an explicit successful policy skip and renders cyan.

## Status meanings

- `PENDING`: the component is not online yet and contributes no progress.
- `OK`, `ONLINE`, `ENABLED`: the component has completed and contributes its full weight.
- `STUB`: the milestone deliberately provides a stub instead of a full implementation; it contributes half of its weight.
- `SKIPPED`: the milestone deliberately skipped optional work; it contributes full weight because the policy decision completed.
- `FAILED`: the component failed and contributes no progress.

## Progress weighting policy

Boot progress is computed from fixed integer weights in the no-heap boot status
model. It is not manually stored in the status structure. The current Boot
Milestone 1.0 policy totals 100 units:

| Component | Weight | Milestone 1.0 policy |
| --- | ---: | --- |
| Architecture | 5 | complete when x86_64 boot setup is active |
| Bootloader | 5 | complete when the Limine handoff is accepted |
| Framebuffer | 6 | complete when framebuffer console is online |
| IDT | 5 | complete after IDT initialization |
| PIC | 5 | complete after PIC initialization |
| Interrupts | 5 | complete after interrupts are enabled |
| Memory | 13 | remains pending until real memory management is online |
| Paging | 13 | remains pending until real paging management is online |
| Heap | 13 | remains pending until the heap allocator is online |
| MTSS | 8 | complete after `kernel.kernel_mtss_init()` |
| Supervisor | 8 | complete after supervisor bootstrap succeeds |
| Root FS | 8 | complete after root filesystem mount succeeds |
| Userspace | 6 | `STUB` contributes 3 units in this milestone |

With Architecture, Bootloader, Framebuffer, IDT, PIC, Interrupts, MTSS,
Supervisor, and Root FS complete and Userspace stubbed, the screen reports 58%:
`5 + 5 + 6 + 5 + 5 + 5 + 8 + 8 + 8 + 3 = 58`.

## Current-stage policy

The current-stage field is a fixed enum (`BootStage`) with static messages, so it
requires no heap allocation. Boot Milestone 1.0 finishes the visible proof path
while Memory, Paging, and Heap remain pending, so the final persistent screen
continues to show `Waiting for Memory Manager` until a later milestone promotes
real memory management.

## Complete in this milestone

- A fixed-size, no-heap boot status model in `src/kernel/boot_status.rs`.
- Weighted progress calculation and static `BootStage` messages.
- Persistent boot-screen rendering in `src/kernel/boot_screen.rs`.
- Change-gated redraws so repeated identical status renders do not spam the framebuffer or serial console.
- x86_64 framebuffer color text support for green/yellow/red/gray/cyan status output.
- Serial fallback using the same plain-text status content plus progress summary.
- x86_64 boot-flow status updates for framebuffer, IDT, PIC, and interrupt enablement.
- Boot-flow updates for root filesystem, supervisor, userspace stub, and MTSS status.
- A QEMU make target for the milestone path.

## Still pending

- Real memory map ownership milestone.
- Physical frame allocator milestone.
- Paging mapper milestone.
- Kernel heap milestone.
- Full userspace launch semantics.
- Full debug shell implementation after the ESC prompt.

## How to run under QEMU

```sh
make milestone-boot-screen
```

The target builds the framebuffer-enabled Mirage image, rebuilds the ISO, runs
QEMU without `-S`, keeps serial on stdio, and writes the QEMU debug log to
`build/qemu.log`.

## Expected framebuffer output

The framebuffer should clear to black and show the GNU/MIRAGE status screen. The
screen should remain visible after early boot logging completes. Status colors:

- `OK`, `ONLINE`, and `ENABLED`: green.
- `PENDING`: yellow.
- `STUB` and `SKIPPED`: cyan.
- `FAILED`: red.
- Labels and prompt: white/gray.
- Progress bar completed cells: green.
- Progress bar incomplete cells: gray.
- Progress percent: yellow while incomplete, green at 100%.

## Expected serial output

Serial stdio should include the same plain-text status block and the progress
summary:

```text
Boot progress: 58%
Current stage: Waiting for Memory Manager
```

ANSI color is not required for serial output in this milestone.

## Next milestones

1. Real memory map ownership.
2. Physical frame allocator.
3. Paging mapper.
4. Kernel heap.
5. Full userspace init.
6. Full debug shell.
