# PID1 Handoff Through MTSS

Mirage launches PID1 as a real userspace task handoff, not as a kernel function call. The canonical PID1 image is `/spider-rt/sbin/spider-rs`, and the dispatcher daemon `/spider-rt/sbin/spider-rsd` is a required runtime follow-on service.

## Required order

```text
bootloader
    -> Mirage kernel mechanisms
    -> RuntimeVfs / rootfs availability
    -> Mirage Supervisor online
    -> MTSS online
    -> userspace loader starts
    -> read /spider-rt/sbin/spider-rs
    -> validate ELF64 x86_64 executable and PT_LOAD mappings
    -> Supervisor authorizes PID1 launch
    -> kernel creates address space/process/thread records
    -> MTSS admits PID1 runnable
    -> architecture backend enters ring 3
    -> spider-rs starts spider-rsd
```

`maybe_launch_pid1` is the boot coordinator gate. It must refuse launch until root filesystem access, Supervisor, MTSS, Spider Runtime availability, loader start, and PID1 image validation are all true. A launch deferred before MTSS comes online must be retried immediately after MTSS transitions online.

## Status truth rules

* `SPIDER-RS [FOUND]` means `/spider-rt/sbin/spider-rs` was read from RuntimeVfs/rootfs.
* `PID1 [CREATED]` means a real kernel process record exists.
* `PID1 [RUNNABLE]` means MTSS admitted a real task/thread to a runnable queue.
* `PID1 [RUNNING]` requires that architecture user-mode execution actually began.
* `SYSTEM DISPATCHER [RUNNING]` requires that real Spider-rs code spawned `spider-rsd`.
* `M1 TERMINAL [RUNNING]` requires real userspace app launch through Spider-rs/spider-rsd, not kernel-authored fake output.

## Supervisor boundary

The Supervisor authorizes Spider-rs as PID1 and records policy approval. It does not directly mutate MTSS run queues. Kernel/MTSS admission performs process/thread creation and runnable insertion. The architecture backend performs the final ring-3 transition.

## Current status

The current documented milestone supports honest PID1 discovery, ELF validation, Supervisor approval, process/task/thread creation, and MTSS runnable admission. The full user-mode transition remains pending. Therefore boot reports must stop at runnable/pending states rather than claiming Spider-rs, spider-rsd, or M1 Terminal are online.

## Failure handling

Failures must be exact and typed: RuntimeVfs unavailable, missing Spider-rs, unsupported ELF, invalid PT_LOAD mapping, stack preflight failure, Supervisor denial, MTSS spawn/admission failure, dispatcher unavailable, or missing user-mode transition. A missing `/spider-rt/sbin/spider-rs` or `/spider-rt/sbin/spider-rsd` is a build failure.
