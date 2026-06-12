# Mirage Boot Phase Manager

The Boot Phase Manager is the canonical subsystem registration and boot-status
system for Mirage Boot Milestone 1.1. No subsystem is allowed to come online
silently: every boot subsystem must register a static descriptor before it
initializes, then report state transitions through `src/kernel/boot_phase.rs`.
The module is exposed through `src/kernel/mod.rs` as `kernel::boot_phase`.

## Subsystem registration rule

Every boot subsystem registers a `SubsystemDescriptor` before initialization:

- `phase`: stable `BootPhase` identity.
- `name`: short framebuffer/serial display label.
- `category`: coarse ownership group.
- `required`: whether failure blocks the intended milestone policy.
- `weight`: progress contribution.

Milestone 1.1 registers all known core descriptors during the seed-rs handoff,
after `.bss` is cleared and before `SeedRs` starts. Registration emits a serial
line and stores the record in a fixed table. Duplicate registration is detected
and reported as a warning instead of silently overwriting the existing record.

If code starts a phase that was not registered, the manager auto-registers the
phase with its static fallback descriptor, prints:

```text
[phase] WARNING: <phase> started without registration
```

and makes the phase visible to boot-screen/debug-shell readers.

## Categories

`SubsystemCategory` groups registered records for rendering and future queries:

- `Seed`
- `Boot`
- `Architecture`
- `Memory`
- `Device`
- `Input`
- `Storage`
- `Supervisor`
- `Userspace`
- `Scheduler`
- `Debug`

The category is metadata only; policy decisions still belong to the supervisor
or the architecture/kernel mechanism that owns the action.

## States

Each registered subsystem reports one of these states:

| State | Meaning |
| --- | --- |
| `Unregistered` | No visible descriptor exists for this slot. This should not be seen for known Milestone 1.1 subsystems after default registration. |
| `Registered` | A descriptor was registered and serially reported. |
| `Pending` | The subsystem is known, visible, and not yet started. |
| `Started` | Initialization is in progress. |
| `Ok` | Initialization completed successfully. |
| `Online` | The subsystem is available for live use. |
| `Enabled` | The subsystem was enabled, such as interrupt delivery. |
| `Failed` | Initialization failed. The record message contains a static diagnostic. |
| `Skipped` | The subsystem is optional or unavailable in this build/environment. |
| `Stub` | A deliberate milestone stub represents future functionality. |

Required failures are rendered red and serially diagnosed. Optional absent
hardware must be marked `Skipped`, not `Failed` (for example, no USB keyboard or
no ACPI EC hotkeys).

## Required vs. optional

`required = true` means the subsystem is part of the intended boot milestone and
its failure is a boot-policy concern. `required = false` means the subsystem is
optional for this milestone; absence should be visible as `Skipped` or `Stub`.
The manager records the fact, but it does not decide whether the system may
continue. Continuing after a required failure remains boot/supervisor policy.

## Weight and progress policy

`boot_phase_progress_percent()` computes progress from registered records only:

- `Ok`, `Online`, `Enabled`: full weight.
- `Skipped`, `Stub`: full weight for optional subsystems, half weight for
  required subsystems.
- `Started`: half weight.
- `Pending`, `Registered`, `Unregistered`: zero.
- `Failed`: zero and the framebuffer progress bar warns in red.

The percentage is:

```text
completed_registered_weight * 100 / total_registered_weight
```

No heap allocation, `Vec`, or `String` is used.

## Framebuffer rendering

`src/kernel/boot_screen.rs` renders from `BootPhaseManager` snapshots. It does
not maintain a separate hand-written status structure; labels and state strings
come from registered subsystem records.

The persistent screen is ordered as:

1. Core seed/boot/architecture/memory records.
2. Supervisor, root filesystem, userspace, and scheduler records.
3. Input status records.
4. Progress and current phase.

Current milestone-visible layout:

```text
GNU/MIRAGE

               Mirage Boot Milestone 1.1

Seed-rs      [ OK ]
BootInfo     [ OK ]
Architecture [ STARTED ]
Serial       [ OK ]
GDT          [ OK ]
Memory       [ OK ]
Paging       [ OK ]
Heap         [ ONLINE ]
Framebuffer  [ ONLINE ]
IDT          [ OK ]
PIC          [ OK ]
Interrupts   [ ENABLED ]

Supervisor   [ PENDING ]
Root FS      [ PENDING ]
Userspace    [ PENDING ]
MTSS         [ PENDING ]

Input        [ PENDING ]
USB Kbd      [ STARTED ]
PS/2 Kbd     [ OK ]
EC Hotkeys   [ PENDING ]

Boot Progress
[##################------------] 58%

Current Phase:
USB Keyboard

Press ESC for debug shell
```

Framebuffer color rules:

- `OK`, `ONLINE`, `ENABLED`: green.
- `STARTED`, `PENDING`: yellow.
- `REGISTERED`: gray.
- `SKIPPED`: gray/cyan.
- `STUB`: cyan.
- `FAILED`: red.
- labels: white/gray.
- `GNU/MIRAGE`: cyan.
- background: black.

Framebuffer redraws are attempted only after `Framebuffer` is `Online`, `Ok`, or
`Enabled`. If the framebuffer is skipped, serial remains the authoritative boot
status path.

## Serial transition logs

Every registration and transition writes a plain COM1 line. Examples:

```text
[phase] Seed-rs: registered
[phase] Memory: STARTED
[phase] Memory: OK
[phase] Supervisor: FAILED: minimal supervisor bootstrap failed
[phase] EC Hotkeys: SKIPPED: EC absent
```

Serial logs intentionally avoid heap formatting and ANSI color requirements.

## Future debug shell query path

The future debug shell should query this same table rather than duplicating boot
state. Read-only commands can use:

- `boot_phase_current()`
- `boot_phase_state(phase)`
- `boot_phase_progress_percent()`
- `boot_phase_records(callback)`
- `boot_phase_snapshot()` for fixed-table rendering

State mutation remains reserved for boot code and subsystem initialization paths;
policy actions such as restarting services or remounting root belong above this
mechanism layer.
