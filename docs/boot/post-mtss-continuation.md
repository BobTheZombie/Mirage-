# Post-MTSS Boot Continuation

Mirage must not stop boot after MTSS is created.  MTSS owns portable task and
thread lifecycle, but it is not the final boot state.  Once MTSS is online, the
boot coordinator resolves every remaining runtime gate and then enters the
kernel idle loop if user-mode dispatch is not yet available.

## Root cause fixed

The boot path created MTSS and marked it online, but the remaining boot gates
were still represented by early deferred/stub statuses.  Those statuses could
leave `Root FS`, `Userspace Loader`, `Spider-rs`, `System Dispatcher`, and
`IdleLoop` looking pending at the common stop point even though MTSS itself had
finished initialization.

The fixed path calls `continue_after_mtss_online` immediately after MTSS becomes
online.  That coordinator uses the live dependency state collected during boot
rather than stale “handoff not reached” messages.

## Continuation order

After `MTSS [Online]`, Mirage now resolves the following sequence:

1. Confirm MTSS is online.
2. Confirm rootfs was resolved by the earlier mount attempt.
3. If rootfs is unavailable, mark userspace and dispatcher gates skipped or
   pending with an exact rootfs reason.
4. If rootfs, supervisor, MTSS, and RuntimeVfs are available, start the
   userspace loader.
5. Read `/spider-rt/sbin/spider-rs` from RuntimeVfs.
6. Ask the Supervisor to authorize Spider-rs as PID1.
7. Let the kernel validate the ELF and admit the task through MTSS.
8. Mark PID1 created/runnable only after MTSS-visible admission succeeds.
9. Mark the dispatcher pending with the exact ring-3 limitation.
10. Enter `IdleLoop [RUNNING]` so the system no longer remains silently pending.

## Rootfs resolution semantics

Rootfs must not remain pending after known sources have been evaluated.

* A successful `mount_root_from_boot_sources` is reported as `ROOT FS [OK]`.
* A failed mount attempt is reported as `ROOT FS [FAILED: no root source configured]`.
* If rootfs is unavailable after MTSS, the userspace loader is marked skipped and
  the dispatcher is marked pending on rootfs availability.

## Boot progress policy

Dispatcher startup is not faked.  Current boot may resolve into an honest
incomplete-userspace state:

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

This is a resolved boot state for the current milestone, not a claim that the
ring-3 dispatcher is online.
