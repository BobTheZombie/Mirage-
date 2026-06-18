# PID1 boot order audit

## Findings

The old boot sequence mounted RuntimeVfs and initialized the Supervisor before MTSS. It then marked userspace as deferred with the message `userspace init launch deferred: root FS and supervisor are online; MTSS handoff not reached yet`. MTSS later initialized, but no code retried the deferred launch.

## Fix

A `BootRuntimeDeps` coordinator now gates PID1 launch on these dependencies:

- root FS online
- Supervisor online
- MTSS online
- Spider Runtime available

`maybe_launch_pid1` is called after the MTSS online transition, so the previous deferred path is retried. The function starts the userspace loader, reads `/spider-rt/sbin/spider-rs`, validates ELF, asks the Supervisor for authorization, and launches through the kernel/MTSS admission path.

## Status honesty

- Spider-rs is `Found` when the RuntimeVfs file is read.
- Spider-rs is `ELF Ok` only after ELF validation succeeds.
- PID1 is `Created` only after process creation succeeds.
- PID1 is `Runnable` only after MTSS scheduler insertion succeeds.
- Dispatcher remains `Pending` because ring-3 transition is not implemented.

No state is marked full userspace `Online` for this milestone.
