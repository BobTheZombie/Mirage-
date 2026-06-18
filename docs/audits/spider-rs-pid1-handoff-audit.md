# Spider-rs PID1 Handoff Audit

## Findings

The stale boot message was emitted before MTSS initialization and said the userspace launch was deferred because “MTSS handoff not reached yet.” After MTSS later became online, that wording remained misleading: MTSS itself was no longer missing. The actual missing stage was the Spider-rs PID1 handoff through the userspace loader and MTSS runnable admission.

## Fix

The boot gate now uses the authoritative `mtss_online` dependency set when `kernel.kernel_mtss_init()` completes. If MTSS is online, the loader no longer reports “MTSS handoff not reached.” It either starts the PID1 handoff or reports an exact missing dependency such as root FS, supervisor, RuntimeVfs, Spider-rs ELF, MTSS spawn, or user-mode transition.

## State machine

The handoff state records these milestones:

- `NotStarted`
- `RuntimeUnavailable`
- `RuntimeFound`
- `ElfValidated`
- `ProcessCreated`
- `MtssTaskCreated`
- `Runnable`
- `DispatcherStarted`
- `DispatcherPending(reason)`
- `Failed(error)`

Typed errors include RuntimeVfs unavailable, missing Spider-rs binary, invalid/unsupported ELF, segment/stack/process failures, supervisor denial, MTSS unavailability/spawn failure, dispatcher unavailability, and missing user-mode transition.

## Implemented vs pending

Implemented: RuntimeVfs lookup, ELF validation, supervisor approval, kernel process record, MTSS task/thread creation, runnable state, and manifest recording for the terminal child.

Pending: real ring-3 transition, dispatcher online confirmation, child spawning, and console ABI.
