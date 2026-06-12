# Mirage Boot Phase Manager

Mirage now tracks early x86_64 boot through a formal, no-heap boot phase
manager instead of relying on scattered numeric boot markers.

The manager lives in `src/kernel/boot_phase.rs` and is exposed by
`src/kernel/mod.rs` as `kernel::boot_phase`.

## Design goals

The boot phase manager is intended for the earliest QEMU/seed-rs boot path:

- `no_std`
- no heap allocation
- no `Vec` or `String`
- fixed-size static phase table
- serial diagnostics before the normal kernel logger is safe
- framebuffer rendering only after the framebuffer phase is online
- queryable state for future debug-shell commands

Serial is authoritative. The framebuffer screen is optional and best-effort.

## Boot phase list

The fixed phase table is ordered as follows:

1. `SeedRs`
2. `BootInfo`
3. `KernelMain`
4. `Architecture`
5. `Serial`
6. `Gdt`
7. `PhysicalAllocator`
8. `KernelMapper`
9. `Heap`
10. `MemoryMap`
11. `Memory`
12. `Framebuffer`
13. `Idt`
14. `Pic`
15. `Interrupts`
16. `KernelConstructed`
17. `BootInfoApplied`
18. `SupervisorCreated`
19. `RootFs`
20. `Supervisor`
21. `Userspace`
22. `Mtss`
23. `BootScreen`
24. `IdleLoop`

These phases intentionally describe Mirage's mechanism/policy boundary: seed-rs
and architecture setup report low-level mechanisms, while root filesystem,
supervisor, userspace, MTSS, boot screen, and idle loop report later boot
milestones.

## Phase states

Each phase has one of these states:

| State | Meaning |
| --- | --- |
| `Pending` | The phase has not been reached. |
| `Started` | The phase is currently being attempted. |
| `Ok` | The phase completed successfully. |
| `Failed` | The phase was attempted and failed. The message field contains a static failure summary. |
| `Skipped` | The phase was deliberately skipped because the current build or boot environment does not provide it. |
| `Stub` | The phase is intentionally represented by a milestone stub. |

Every transition emits a concise raw COM1 diagnostic such as:

```text
[phase] Memory: started
[phase] Memory: ok
[phase] Supervisor: failed: full service manifest incomplete
```

## Progress calculation

Progress is computed from the fixed table without allocation:

- `Pending` = 0 units
- `Started` = 1 unit
- `Ok` = 2 units
- `Skipped` = 2 units
- `Stub` = 2 units
- `Failed` = 0 units

The percentage is:

```text
sum(phase units) * 100 / (phase_count * 2)
```

`Skipped` and `Stub` count as complete for milestone progress because they are
explicit, known boot outcomes rather than unresolved work. Failed phases count
as zero and cause the framebuffer progress bar to render in red.

## Framebuffer integration

`kernel::boot_screen` renders from `BootPhaseManager` snapshots. It no longer
uses manually maintained boot-screen status fields.

The persistent screen displays:

```text
GNU/MIRAGE

Mirage Boot Milestone 1.1

Seed-rs        [ OK ]
BootInfo       [ OK ]
Architecture   [ OK ]
Serial         [ OK ]
GDT            [ OK ]
Memory         [ OK ]
Paging         [ OK ]
Heap           [ ONLINE ]
Framebuffer    [ ONLINE ]
IDT            [ OK ]
PIC            [ OK ]
Interrupts     [ ENABLED ]
Supervisor     [ OK/PENDING/FAILED ]
Root FS        [ OK/PENDING/FAILED ]
Userspace      [ STUB/PENDING ]
MTSS           [ OK/PENDING ]

Boot Progress
[###############-------------] 76%

Current Phase:
<phase name>

Press ESC for debug shell
```

Color policy:

- `Ok`, `ONLINE`, and `ENABLED`: green
- `Pending` and `Started`: yellow
- `Stub` and `Skipped`: cyan
- `Failed`: red

Framebuffer rendering is attempted only after `Framebuffer` reaches `Ok`. If no
framebuffer is provided, or if the build does not enable framebuffer support,
the `Framebuffer` phase becomes `Skipped` and serial remains the only output.

## Serial fallback

The manager writes transition diagnostics with raw x86_64 COM1 routines instead
of `kprintln!`. This keeps seed-rs and early boot diagnostics independent of the
normal kernel console, framebuffer mirroring, allocator state, supervisor state,
or MTSS.

The older seed-rs textual markers remain useful compatibility aliases, but the
canonical failure locator is the last `[phase] ...` transition printed on COM1.

## Identifying a failure point

When boot hangs or halts:

1. Read the final `[phase]` line on the serial console.
2. If the final state is `Started`, that phase is the active suspect.
3. If the final state is `Ok`, the next pending phase in the fixed table is the
   next likely code path.
4. If the final state is `Failed`, the static message after `failed:` is the
   formal failure summary.
5. If the framebuffer is online, check `Current Phase:` and the red/yellow/green
   table for the same information.

For example, if QEMU stops immediately after:

```text
[phase] Interrupts: ok
```

then interrupt enabling completed and the next visible phase should be
`Architecture: ok` or `KernelConstructed: ok`, depending on where execution
stops after returning from `x86_64::init_architecture`.

## Current integration points

- seed-rs marks `SeedRs` and `BootInfo`.
- `kernel_main` marks `KernelMain`, `Architecture`, `KernelConstructed`,
  `BootInfoApplied`, `SupervisorCreated`, `RootFs`, `Supervisor`, `Userspace`,
  `Mtss`, `BootScreen`, and `IdleLoop`.
- x86_64 architecture setup marks `Serial`, `Gdt`, `MemoryMap`,
  `KernelMapper`, `PhysicalAllocator`, `Heap`, `Memory`, `Framebuffer`, `Idt`,
  `Pic`, and `Interrupts`.

## Future debug-shell integration

The debug shell can later expose read-only commands that call:

- `boot_phase_current()`
- `boot_phase_state(phase)`
- `boot_phase_progress_percent()`
- a snapshot/table rendering helper based on `boot_phase_snapshot()`

Those commands should remain read-only. Policy decisions such as retrying a
service, remounting root, or launching userspace belong to the supervisor, not
the boot phase manager.
