# Mirage MTSS Architecture

MTSS is the Mirage Multitasking Subsystem. It owns portable task, process, micro-thread, run-queue, scheduler accounting, lifecycle, and scheduler-visible state mechanics. It does **not** own CPU privilege transitions, raw interrupt entry, `iretq`/`sysret`, page-table installation, capability grant policy, service authorization, or crash-recovery policy.

## Architectural position

```text
Applications / GNU-POSIX compatibility
    -> Mirage runtime / libc
    -> IPC + capability checks
    -> Mirage Supervisor policy
    -> MTSS portable scheduling mechanics
    -> Mirage kernel mechanisms
    -> architecture backend / hardware
```

The boundary is deliberate:

* **Kernel:** owns CPU entry/exit, interrupt/trap entry, hardware timer delivery, virtual memory mechanisms, page tables, low-level IPC transport, and capability enforcement.
* **MTSS:** owns runnable-state management, task/thread lifecycle transitions, scheduler-visible identities, FIFO run-queue mechanics, time-slice accounting contracts, and lifecycle events.
* **Supervisor:** owns policy: service launch authorization, driver lifecycle, crash detection/recovery, capability grant/revoke decisions, and boot ordering.

## Scheduler model

The current MTSS core is an allocation-free, fixed-capacity scheduler skeleton suitable for early `no_std` boot. It provides:

* stable task, thread, process, address-space, CPU, and run-queue identifiers;
* scheduler-visible `TaskState`, `ThreadState`, and `ProcessState` enums;
* a fixed-capacity FIFO run queue;
* scheduler accounting counters for admissions, completions, context switches, preemptions, blocking, sleep/wake, and containment;
* backend traits for CPU identification, clock sources, timer arming, lifecycle sinks, statistics sinks, and thread-state storage;
* a separate `CoreMtss` path for the first userspace launch milestone.

Policy-neutral priority values exist only as hints. MTSS records them, but it does not decide service importance, user policy, or recovery behavior.

## Core task/thread model

The early userspace milestone uses `CoreMtss`:

* PID/TID allocation starts at `1`; idle task/thread is `0`.
* Idle is a kernel task with a per-CPU idle thread.
* Userspace task creation requires a non-zero address-space ID and CR3, canonical entry, valid user stack, valid kernel stack, and complete preflight proof.
* A task is inserted as `Created` and its main thread as `New`; only after preflight succeeds is the task changed to `Runnable`, the thread changed to `Ready`, and the thread enqueued.
* Timer ticks call `on_timer_tick()`, which delegates to `schedule_next()`.
* `schedule_next()` returns a running non-idle thread when one is ready; otherwise it selects the idle thread.
* `exit_current()` marks the current thread `Zombie` and the task `Exited`.

## State model

Current portable MTSS states are intentionally small:

| Object | States |
| --- | --- |
| Task | `Created`, `Runnable`, `Running`, `Blocked`, `Exited` |
| Thread | `New`, `Ready`, `Running`, `Blocked`, `Sleeping`, `Zombie`, `Dead` |
| Process | `New`, `Ready`, `Running`, `Waiting`, `Zombie`, `Dead`, `Failed` |

The architectural target includes richer lifecycle concepts such as contained, terminated, and reaped. Those concepts belong in MTSS-visible lifecycle events and future process/service integration, while recovery decisions remain supervisor policy.

## Readiness vocabulary

MTSS readiness is reported in layers rather than as a single ambiguous boolean:

* **Core readiness** means portable task/thread/process records, idle identity, lifecycle states, accounting storage, and admission preflight APIs are initialized.
* **Scheduler readiness** means ready-queue insertion, non-idle selection, idle fallback, and truthful Created/New to Runnable/Ready transitions are working.
* **Timer readiness** means a bounded kernel/architecture timer source can deliver scheduler ticks to MTSS without MTSS owning raw hardware timer programming.
* **Preemption readiness** means timer delivery can save the active architecture context, enter MTSS scheduling hooks, restore the selected context, and return safely.
* **Degraded readiness** means core/scheduler/idle/API readiness is valid but full timer/preemption proof is absent, so MTSS is cooperative or boot-stage only.
* **Online readiness** means full preemptive scheduling is proven. `MTSS ONLINE` means this state only; it must not describe degraded or cooperative MTSS.

A degraded/cooperative MTSS may still create tasks and mark PID1 runnable when core, scheduler, idle, and admission API readiness are valid and Supervisor policy allows it. The boot policy switch `require_preemption_for_userspace` defaults to `false`; setting it to `true` blocks PID1 handoff until preemption readiness is proven. Boot coordination must retry PID1 launch after MTSS readiness state changes so pending cooperative/preemptive policy decisions do not become stale. See [`mtss-readiness.md`](mtss-readiness.md).

## Supervisor boundary

MTSS may expose events like task created, thread runnable, thread running, blocked, sleeping, faulted, contained, terminated, and reaped. The Supervisor may observe those events and decide whether to restart a service, revoke capabilities, or report boot failure. The Supervisor must not directly mutate MTSS queues, fabricate task state, or mark PID1/service status as online unless the corresponding real MTSS/kernel path executed.

## Limitations

MTSS is currently a bootable architecture skeleton, not a production scheduler:

* SMP load balancing is not complete.
* Priority policy is not production scheduling policy.
* Full blocked/sleeping wake-source plumbing is incomplete.
* Full process hierarchy/reaping integration is still being built.
* Real ring-3 PID1 dispatch is still an architecture/backend handoff blocker; Mirage can create/runnable-admit PID1 but must not claim userspace execution until the real transition occurs.

See also:

* [`mtss-readiness.md`](mtss-readiness.md)
* [`mtss-preemption.md`](mtss-preemption.md)
* [`mtss-process-lifecycle.md`](mtss-process-lifecycle.md)
* [`../boot/pid1-handoff.md`](../boot/pid1-handoff.md)
* [`../audits/mtss-production-audit.md`](../audits/mtss-production-audit.md)

## External scheduler references

The Zinnia audit reinforced Mirage's existing MTSS boundary: useful scheduler ideas are explicit state transitions, idle task fallback, runnable queues, preemption requests, and reaping/accounting separation. Mirage reimplements these ideas only inside MTSS-owned code. The lower kernel still owns timer interrupt delivery and CPU context mechanics, and the Supervisor still owns launch/recovery policy.
