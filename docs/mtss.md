# Mirage MTSS core and first userspace task handoff

MTSS is the Mirage Multitasking Subsystem. It owns portable task/thread lifecycle mechanics, scheduler-visible state, task and thread identifiers, ready queues, timer-tick scheduling hooks, and safe task-exit bookkeeping. It does **not** own Supervisor policy, capability grants, CR3 installation, trap entry, `iretq`, raw timer programming, or service authorization.

## Boundary

The first Spider-rs launch path keeps the Mirage architecture split intact:

```text
Mirage-dispatch-rs starts kernel components/subsystems
  -> Supervisor authorizes service launch
  -> MTSS creates and schedules task/thread objects
  -> x86_64 backend enters userspace
  -> Spider-rs runs as userspace PID 1
```

Spider-rs must never be called as a kernel Rust function. MTSS creates a userspace task object and a main thread with an address-space handle, user stack range, kernel stack range, and CPU context. The architecture backend is responsible for the actual ring-3 transition.

## Current implementation

`crates/mirage-mtss/src/task_core.rs` adds an allocation-free core table for the first userspace milestone:

- fixed-capacity task table;
- fixed-capacity thread table;
- FIFO ready queue;
- `CoreTaskId` PID allocation starting at PID 1 after the idle task;
- `TaskKind::{Kernel, Userspace}`;
- `CoreTaskState::{New, Ready, Running, Blocked, Sleeping, Exited, Faulted}`;
- `StackRange`, `SavedRegisters`, and `CpuContext` scaffolding;
- idle task initialization;
- userspace task creation through `CoreMtss::spawn_userspace`;
- timer hook through `CoreMtss::on_timer_tick`;
- safe current-thread exit through `CoreMtss::exit_current`.

The existing MTSS scheduler facade remains available for current kernel integrations. The new core is deliberately separate so the PID 1 milestone can progress without rewriting existing scheduler tests.

## Current limitation

The kernel boot path marks Spider-rs as `Stub` unless a real ELF/rootfs byte path and architecture userspace entry path are available. This avoids falsely claiming that userspace entered ring 3.

## Userspace init bring-up note (2026-06-12)

This pass keeps MTSS honest for QEMU userspace-init bring-up: the kernel can mark MTSS `Online` once the scheduler skeleton is initialized, but Spider-rs remains blocked on the missing user address-space and ring-3 entry backend. Persistent loops such as `IdleLoop` should transition to `Running` after they enter their loop.
