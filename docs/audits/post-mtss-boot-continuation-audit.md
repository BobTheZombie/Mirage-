# Post-MTSS Boot Continuation Audit

## Audit summary

The audited boot path had already completed low-level architecture bring-up,
Supervisor initialization, and MTSS initialization.  The remaining common stop
was after `MTSS initialized`/`MTSS [Online]`, with runtime gates still reported
as pending or as stale pre-MTSS handoff stubs.

## Root cause

The boot code did not have a single post-MTSS dependency coordinator.  Rootfs,
userspace loader, Spider-rs PID1, system dispatcher, and idle loop states were
updated in separate places, and early messages such as “waiting for MTSS online”
could survive past the point where MTSS was already online.

## Fix

A post-MTSS continuation function now runs immediately after MTSS transitions
online.  It consumes the boot dependency record and resolves the remaining gates:

* rootfs resolved/online state
* Supervisor online state
* MTSS online state
* RuntimeVfs availability
* userspace loader start
* Spider-rs discovery and ELF validation
* PID1 process creation and MTSS runnable admission
* dispatcher pending reason
* idle loop entry

## Before

```text
MTSS [ONLINE]
ROOT FS [PENDING]
USERSPACE LOADER [PENDING]
SPIDER-RS [PENDING/STUB]
SYSTEM DISPATCHER [PENDING]
IDLELOOP [PENDING]
```

## After

Expected current-milestone result when RuntimeVfs and rootfs are available:

```text
MTSS [Online]
ROOT FS [OK]
USERSPACE LOADER [STARTED]
SPIDER-RS [FOUND]
SPIDER-RS [ELF OK]
PID1 [CREATED]
PID1 [RUNNABLE]
SYSTEM DISPATCHER [PENDING: user-mode transition not implemented]
IDLELOOP [RUNNING]
```

If rootfs is unavailable, the loader and dispatcher receive exact skipped or
pending reasons and `IDLELOOP [RUNNING]` is still reached.

## Remaining limitations

* The dispatcher is not marked started because ring-3 transition is still not
  implemented.
* `mirage-m1-terminal` remains a future Spider-rs child application, not PID1.
* Hardware-specific QEMU/VirtualBox verification must confirm the serial/boot UI
  reaches the resolved post-MTSS markers on each target.
