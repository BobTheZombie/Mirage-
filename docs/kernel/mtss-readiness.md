# MTSS Readiness and PID1 Eligibility

This document defines the readiness vocabulary used by boot code, diagnostics, and audits when deciding whether MTSS may admit PID1.

## Readiness terms

| Term | Meaning | PID1 eligibility |
| --- | --- | --- |
| **MTSS core readiness** | The portable task/thread/process structures, fixed-capacity records, idle identity, lifecycle states, accounting storage, and admission preflight API are initialized. | Required. |
| **MTSS scheduler readiness** | At least one truthful scheduler path exists: ready-queue insertion, non-idle selection when runnable work exists, idle fallback when no work exists, and explicit Created/New to Runnable/Ready transitions. | Required. |
| **MTSS timer readiness** | The kernel or architecture backend can deliver bounded timer ticks to MTSS hooks and record scheduler time without MTSS programming raw hardware timers. | Required for preemptive online, not required for cooperative PID1 admission unless policy says otherwise. |
| **MTSS preemption readiness** | A hardware timer interrupt can preempt the current context, save the architecture frame, call MTSS scheduling hooks, restore the selected context, and return without corrupting kernel or user state. | Required for `MTSS ONLINE` and preemptive PID1 handoff. |
| **MTSS degraded readiness** | Core, scheduler, idle, and public admission APIs are valid, but timer/preemption proof is missing or intentionally disabled. Scheduling is cooperative or boot-stage only. | May create tasks and mark PID1 runnable when userspace policy allows cooperative MTSS. |
| **MTSS online readiness** | Full preemptive scheduling is proven: core + scheduler + timer + preemption + architecture context restore are valid for the target. | Allows preemptive PID1 handoff and may be reported as `MTSS ONLINE`. |

## Meaning of `MTSS ONLINE`

`MTSS ONLINE` is reserved for full preemptive scheduling readiness. It must not be used for a merely initialized scheduler, a cooperative-only run queue, or a boot-stage admission path. If preemption is absent, diagnostics must say degraded/cooperative and must not collapse that state into online.

## Degraded/cooperative PID1 admission

A degraded/cooperative MTSS may create tasks, create threads, and mark PID1 runnable if all of the following are true:

1. MTSS core readiness is valid.
2. MTSS scheduler readiness is valid.
3. The idle thread/path exists as a truthful fallback.
4. The task admission API validates address-space, CR3, entry, stack, kernel stack, and ELF preflight proof.
5. The Supervisor has authorized PID1 launch.
6. Boot policy does not require preemption before userspace.

This state means `PID1 [RUNNABLE]` is truthful, but it does not mean PID1 has run, that ring-3 entry happened, or that `MTSS ONLINE` is true.

## `require_preemption_for_userspace`

`require_preemption_for_userspace` is the boot policy switch that decides whether cooperative MTSS may admit PID1. Its default is `false` for the current architecture skeleton so PID1 can be created and admitted runnable after core/scheduler/idle/API readiness even while full preemption remains pending.

When `require_preemption_for_userspace = true`, PID1 handoff must wait until MTSS preemption readiness is valid. In that configuration, degraded/cooperative readiness is insufficient for userspace admission even though task creation APIs may be initialized.

## Retry behavior after MTSS state changes

`maybe_launch_pid1` must be retried after every material MTSS readiness transition: core initialized, scheduler ready, degraded/cooperative readiness reached, timer readiness reached, preemption readiness reached, or online readiness reached. A previous pending result must not become stale. If the only blocker was policy requiring preemption, the retry after preemption readiness must either launch PID1 or report the next exact blocker.

## PID1 handoff status strings

* `PID1 HANDOFF [ALLOWED: cooperative MTSS]` means MTSS core/scheduler/idle/API readiness is valid, Supervisor/rootfs/loader preconditions are satisfied, `require_preemption_for_userspace = false`, and PID1 may be created/admitted runnable even though `MTSS ONLINE` is not true.
* `PID1 HANDOFF [ALLOWED: preemptive MTSS]` means full preemptive MTSS readiness is valid and PID1 may be launched under the normal online scheduler contract.
* `PID1 HANDOFF [PENDING: policy requires preemption before userspace]` means cooperative readiness may exist, but `require_preemption_for_userspace = true` and preemption readiness has not been proven yet.

## Full boot audit update

`ONLINE` means core, scheduler, timer, preemption, idle task, task creation, and mark-runnable are all ready. `DEGRADED` means cooperative scheduling is usable while timer/preemption are pending. The default userspace policy allows PID1 handoff in degraded cooperative mode when scheduler-ready.
