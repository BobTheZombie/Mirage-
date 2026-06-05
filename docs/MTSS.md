# Mirage MTSS Ownership

MTSS is the Mirage Micro-Thread Scheduling Service layer. It exists to keep
portable task, thread, and scheduler mechanics out of both the CPU-facing kernel
and the policy-facing supervisor.

MTSS is not a separate policy authority. It is the portable multitasking core
that records scheduler-visible state and chooses backend-neutral state
transitions. CPU-specific execution remains below it, and service lifecycle
policy remains above it.

## What MTSS owns

MTSS owns portable multitasking mechanics:

* task and micro-thread lifecycle state;
* runnable, running, blocked, sleeping, contained, terminated, exited, and
  reaped state transitions;
* run queues and run-queue transitions;
* scheduler-visible task/thread identities;
* portable priority and timeslice accounting;
* scheduler lifecycle events that can be observed by the supervisor or tests;
* backend-neutral scheduling contracts;
* portable scheduler statistics and accounting snapshots.

The implementation should stay backend-neutral. MTSS may decide which
micro-thread record is next runnable, but it must not assume an x86_64 trap
frame layout, a particular interrupt controller, a particular page-table format,
or a supervisor recovery policy.

## What the kernel owns

The kernel owns CPU-facing mechanism and hardware privilege boundaries:

* CPU entry and exit;
* trap, exception, interrupt, and syscall entry/exit;
* low-level context save/restore mechanics;
* low-level timer delivery and preemption hooks;
* address-space and page-table switching;
* virtual memory enforcement;
* IPC transport enforcement;
* capability enforcement;
* low-level module loading;
* hardware privilege boundaries.

The kernel may expose narrow primitives that MTSS uses to enter, leave, or
preempt CPU execution. It should not embed portable run-queue policy,
backend-neutral lifecycle state machines, or service recovery decisions.

## What the supervisor owns

The supervisor owns policy, authority, and recovery:

* service lifecycle policy;
* launch authorization;
* driver-service crash detection and recovery;
* capability grant, revoke, and cleanup policy;
* service registration and discovery policy;
* signed module validation policy;
* boot ordering;
* session management;
* requests to change task priority, contain a task, terminate a task, or reap a
  failed task.

The supervisor may approve or request MTSS state transitions through an explicit
boundary, but it must not inspect or mutate MTSS run queues directly. Recovery
logic remains supervisor policy; portable runnable-state mechanics remain MTSS
mechanism.

## Why scheduler code moved out of the kernel

Scheduler code moved out of the kernel to preserve Mirage's mechanism/policy
split and prevent the kernel from growing into a monolith.

A kernel-local scheduler tends to mix unrelated responsibilities:

```text
CPU trap/timer mechanics
    + portable run queues
    + task lifecycle state
    + priority and timeslice policy
    + supervisor recovery decisions
```

Mirage keeps those seams explicit instead:

```text
supervisor policy request
    -> MTSS portable task/thread/scheduler transition
    -> kernel CPU-entry, context, timer, and privilege mechanism
    -> CPU-specific backend details
```

This move makes the early kernel smaller, makes scheduler state testable without
hardware assumptions, and gives future architectures a stable place to plug in
CPU-specific execution backends without duplicating portable scheduler logic.

## Future CPU-specific backend plan

MTSS will keep portable scheduling logic in shared code while CPU-specific
backends provide the hardware-facing pieces.

Planned backend responsibilities include:

* initialize per-CPU scheduler backend state;
* identify the current CPU;
* read a monotonic scheduler time source;
* arm timer or preemption events for a selected timeslice;
* save and restore CPU context using the architecture ABI;
* enter a selected thread's CPU context;
* return trap, syscall, preemption, yield, or fault outcomes back to the kernel
  and MTSS integration layer.

The expected shape is:

```text
MTSS portable scheduler core
    -> backend-neutral decision record
    -> kernel integration seam
    -> arch backend (x86_64 first, later other CPUs)
    -> trap/syscall/timer outcome
    -> MTSS lifecycle/accounting update
```

The first backend target is x86_64 because Mirage already has x86_64 entry and
context scaffolding. Later CPU ports should reuse MTSS task/thread/run-queue
logic and only replace the architecture-specific context, timer, and CPU-entry
code.

## Non-goals for this milestone

This milestone does not attempt to provide:

* a production SMP scheduler;
* complete preemptive multitasking across real CPUs;
* a final priority-inheritance or real-time scheduling policy;
* CPU-load balancing or NUMA placement;
* complete POSIX signal or pthread semantics;
* a production wait/reap model;
* hardware-specific scheduler optimizations;
* direct supervisor mutation of MTSS run queues;
* direct MTSS ownership of page tables, traps, interrupts, or capabilities;
* direct MTSS ownership of service recovery policy.

The goal is a clean ownership boundary: MTSS owns portable multitasking
mechanics, the kernel owns CPU and privilege mechanisms, and the supervisor owns
policy and recovery.
