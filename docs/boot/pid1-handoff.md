# PID1 Handoff Through KSO and MTSS

Mirage launches PID1 as a real userspace task handoff, not as a kernel function call. The canonical PID1 image is `/spider-rt/sbin/spider-rs`, and the dispatcher daemon `/spider-rt/sbin/spider-rsd` is a required runtime follow-on service.

## Required order

```text
bootloader
    -> Mirage kernel mechanisms
    -> KSO startup policy graph
    -> RuntimeVfs / rootfs availability
    -> Mirage Supervisor launch authorization available
    -> MTSS PID0 readiness gate
    -> userspace loader starts
    -> read /spider-rt/sbin/spider-rs
    -> validate ELF64 x86_64 executable and PT_LOAD mappings
    -> Supervisor authorizes PID1 launch
    -> kernel creates address space/process/thread records
    -> MTSS admits PID1 runnable
    -> architecture backend enters ring 3
    -> spider-rs starts spider-rsd
```

KSO owns the dependency policy that decides when the PID1 handoff gate can be evaluated. `maybe_launch_pid1` remains the concrete boot coordinator gate, but it must be driven by KSO/BootPhase facts rather than a hardcoded single readiness flag. It must refuse launch until root filesystem access, Supervisor approval, Spider Runtime availability, loader start, PID1 image validation, and an eligible MTSS readiness state are all true.

A launch deferred before MTSS eligibility must be retried immediately after any MTSS state change: core initialization, scheduler readiness, degraded/cooperative readiness, timer readiness, preemption readiness, or full online readiness. KSO retry also runs when RuntimeVfs, Supervisor authorization, or loader readiness changes.

## MTSS PID0 eligibility policy

MTSS is PID0, the kernel execution root for scheduling mechanics. PID1 handoff does not always have to wait for full `MTSS ONLINE`. `MTSS ONLINE` means full preemptive scheduling only: MTSS core, scheduler, timer, preemption, and architecture context restore are all proven. Stale wording that says PID1 must wait for `MTSS online` should be read as "PID1 must wait for an eligible MTSS readiness state."

A degraded/cooperative MTSS may create PID1 task/thread records and mark PID1 runnable when core readiness, scheduler readiness, idle fallback, and admission APIs are valid. This is allowed only when the KSO policy switch `require_preemption_for_userspace` is `false`, which is the default for the current architecture skeleton. If `require_preemption_for_userspace` is `true`, cooperative readiness is insufficient and PID1 handoff waits for preemption readiness.

Exact handoff statuses are:

* `PID1 HANDOFF [ALLOWED: cooperative MTSS]` — core/scheduler/idle/API readiness is valid, policy permits cooperative userspace admission, and PID1 may be created/admitted runnable without claiming `MTSS ONLINE`.
* `PID1 HANDOFF [ALLOWED: preemptive MTSS]` — full preemptive readiness is valid, so PID1 may launch under the `MTSS ONLINE` contract.
* `PID1 HANDOFF [PENDING: policy requires preemption before userspace]` — cooperative readiness may be present, but policy requires preemption and preemption readiness is not proven yet.

## Status truth rules

* `SPIDER-RS [FOUND]` means `/spider-rt/sbin/spider-rs` was read from RuntimeVfs/rootfs.
* `PID1 [CREATED]` means a real kernel process record exists.
* `PID1 [RUNNABLE]` means MTSS admitted a real task/thread to a runnable queue.
* `PID1 [RUNNING]` requires that architecture user-mode execution actually began.
* `SYSTEM DISPATCHER [RUNNING]` requires that real Spider-rs code spawned `spider-rsd`.
* `M1 TERMINAL [RUNNING]` requires real userspace app launch through Spider-rs/spider-rsd, not kernel-authored fake output.

## Supervisor boundary

The Supervisor authorizes Spider-rs as PID1 and records policy approval. It does not directly mutate MTSS run queues. KSO waits for the authorization capability, kernel/MTSS admission performs process/thread creation and runnable insertion, and the architecture backend performs the final ring-3 transition.

## Current status

The current documented milestone supports honest PID1 discovery, ELF validation, Supervisor approval, process/task/thread creation, and MTSS runnable admission. The full user-mode transition remains pending. Therefore boot reports must stop at runnable/pending states rather than claiming Spider-rs, spider-rsd, or M1 Terminal are online.

## Failure handling

Failures must be exact and typed: RuntimeVfs unavailable, missing Spider-rs, missing Spider-rsd, unsupported ELF, invalid PT_LOAD mapping, stack preflight failure, Supervisor denial, KSO dependency blocked, MTSS spawn/admission failure, dispatcher unavailable, or missing user-mode transition. A missing `/spider-rt/sbin/spider-rs` or `/spider-rt/sbin/spider-rsd` is a build failure.

## Full boot audit update

PID1 handoff also requires the post-`KernelConstructed` continuation edge to reach KSO graph evaluation, boot-info application, supervisor creation, boot-runtime validation, rootfs mount, MTSS readiness evaluation, userspace loader start, spider-rs ELF preflight, supervisor launch authorization, and MTSS runnable admission. PID1 may be marked runnable only after MTSS accepts the real task/thread into a runnable queue. PID1 may be marked running only after actual user execution begins.
