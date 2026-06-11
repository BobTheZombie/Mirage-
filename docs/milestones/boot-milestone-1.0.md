# Mirage Boot Milestone 1.0 — Persistent Boot Status Screen

## Milestone name

**Mirage Boot Milestone 1.0 — Persistent Boot Status Screen**

This milestone makes the first post-framebuffer boot state visible as an official
GNU/Mirage screen instead of relying only on scrolling early diagnostic logs.

## Screenshot target

The QEMU framebuffer should remain on a black screen with aligned white/gray
labels and colored status words:

```text
GNU/MIRAGE

Mirage kernel boot complete

Architecture : x86_64
Bootloader   : Limine
Framebuffer  : Online
Resolution   : 1024x768x32
IDT          : OK
PIC          : OK
Interrupts   : Enabled
Memory       : Pending
Paging       : Pending
Heap         : Pending
MTSS         : Pending
Supervisor   : Pending

Press ESC for debug shell
```

Actual resolution is supplied by Limine and may differ if QEMU is configured for
a different mode.

## Required statuses

| Subsystem | Required display | Color |
| --- | --- | --- |
| Architecture | `x86_64` once x86_64 boot setup is active | green status color |
| Bootloader | `Limine` after the Limine handoff is accepted | green status color |
| Framebuffer | `Online` when the Limine framebuffer console initializes | green |
| Resolution | `<width>x<height>x<bpp>` from framebuffer metadata | green |
| IDT | `OK` after `idt::initialize()` | green |
| PIC | `OK` after `pic::initialize()` | green |
| Interrupts | `Enabled` after `interrupts::enable()` | green |
| Memory | `Pending` for this milestone | yellow |
| Paging | `Pending` for this milestone | yellow |
| Heap | `Pending` for this milestone | yellow |
| MTSS | `Pending` until the MTSS milestone makes it part of the official boot screen | yellow |
| Supervisor | `Pending` until supervisor bring-up is promoted into a later boot milestone | yellow |

`Failed` is reserved for later fault reporting and must render red when used.

## Complete in this milestone

- A fixed-size, no-heap boot status model in `src/kernel/boot_status.rs`.
- Persistent boot-screen rendering in `src/kernel/boot_screen.rs`.
- x86_64 framebuffer color text support for green/yellow/red/gray/cyan status output.
- Serial fallback using the same plain-text status content.
- x86_64 boot-flow status updates for framebuffer, IDT, PIC, and interrupt enablement.
- A QEMU make target for the milestone path.

## Still pending

- Real memory-map ownership milestone.
- Physical frame allocator milestone.
- Paging mapper milestone.
- Kernel heap milestone.
- MTSS bring-up milestone.
- Supervisor bring-up milestone.
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

- `OK`, `Online`, and `Enabled`: green.
- `Pending`: yellow.
- `Failed`: red if a later boot path uses it.
- Labels and prompt: white/gray.

## Expected serial output

Serial stdio should include the same plain-text status block. ANSI color is not
required for serial output in this milestone.

## Next milestones

1. Real memory map ownership.
2. Physical frame allocator.
3. Paging mapper.
4. Kernel heap.
5. MTSS bring-up.
6. Supervisor bring-up.
