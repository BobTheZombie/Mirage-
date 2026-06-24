# MTSS Production Audit

This audit records the production-readiness state of Mirage MTSS and PID1 handoff documentation.

## Executive summary

MTSS is correctly positioned as the portable multitasking layer between the mechanism-only kernel and the policy-owning Supervisor. It is not production-complete. The current implementation is an architecture skeleton that can model tasks, threads, run queues, timer-tick scheduling hooks, context records, and PID1 runnable admission, while honestly leaving full ring-3 execution and complete process/service lifecycle integration pending.

## Implemented / documented

* MTSS owns portable scheduler identities, state, lifecycle events, run-queue transitions, and accounting.
* Kernel/architecture owns interrupt entry, timer interrupt observation, CR3 switching, TSS/RSP0 setup, and context restore.
* Supervisor owns service launch policy, capability decisions, crash recovery, and boot ordering.
* Userspace task creation has preflight requirements for entry, stack, address-space, CR3, mappings, selectors, kernel stack, and TSS state.
* PID1 handoff uses `/spider-rt/sbin/spider-rs` and must report created/runnable/running states honestly.

## Current limitations

| Area | Status | Production blocker |
| --- | --- | --- |
| Scheduler policy | FIFO skeleton | Needs production priority/fairness policy without moving policy into kernel. |
| SMP | Early/single-core focused | Needs per-CPU queues, load balancing, and synchronization strategy. |
| Preemption | Timer tick and context contract documented | Needs fully proven hardware preemption path across syscall/timer/user return cases. |
| Ring-3 PID1 | Runnable admission documented | Needs completed mapped ELF entry/stack and successful architecture user-mode transition. |
| Process lifecycle | Records and state model exist | Needs full wait/reap, parent/child notification, resource cleanup, and service integration. |
| Blocking/sleeping | States exist | Needs wake-source integration for IPC/futex/timer/supervisor events. |
| Recovery | Boundary documented | Needs end-to-end supervised service crash/revoke/restart demo. |

## QEMU/test results to maintain

Every MTSS/PID1 handoff PR should record fresh results for:

1. `cargo test` or crate-scoped MTSS tests for state transitions, run queue behavior, and preflight validation.
2. boot image validation with `MIRAGE_REUSE_IMAGE=0` proving the required runtime files exist:
   * `/spider-rt/sbin/spider-rs`
   * `/spider-rt/sbin/spider-rsd`
   * `/usr/bin/m1-terminal`
   * `/etc/spider/units/default.target`
   * `/etc/spider/units/basic.target`
   * `/etc/spider/units/m1-terminal.service`
3. QEMU boot logs showing MTSS online and PID1 status reaching only the states actually executed.
4. Negative validation proof that a missing Spider runtime binary fails the build.

This documentation-only audit does not claim a new QEMU run. The next implementation PR must paste the exact commands and acceptance markers.

## Acceptance markers

Acceptable current markers:

```text
MTSS [Online]
SPIDER-RS [FOUND]
PID1 [CREATED]
PID1 [RUNNABLE]
SYSTEM DISPATCHER [PENDING: user-mode transition not implemented]
```

Unacceptable unless backed by real execution:

```text
PID1 [RUNNING]
SPIDER-RSD [RUNNING]
M1 TERMINAL [RUNNING]
```

## Risks

* Accidentally treating PID1 runnable admission as user-mode execution.
* Moving scheduler or service policy into low-level interrupt/kernel code.
* Marking driver services online without real initialization.
* Testing cached images instead of rebuilding with `MIRAGE_REUSE_IMAGE=0`.
* Allowing bootdiag raw text to replace the live milestone framebuffer UI.
