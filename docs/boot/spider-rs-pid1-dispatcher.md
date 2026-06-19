# Spider-rs PID1 Dispatcher

`spider-rs` is Mirage's PID1 system dispatcher/daemon.  It is not the
hello-world application and must not be replaced by a terminal demo.  Later test
applications, including terminal smoke programs, are children that Spider-rs may
launch once dispatcher child launch and the userspace console ABI exist.

## Authority boundary

The Supervisor owns the policy decision to authorize Spider-rs as PID1.  The
kernel owns ELF validation, low-level mapping/admission mechanics, and the
architecture transition.  MTSS owns task/thread lifecycle and runnable state.
The Supervisor must not directly mutate MTSS queues.

The intended chain is:

```text
RuntimeVfs /spider-rt/sbin/spider-rs
    -> Supervisor policy authorization
    -> kernel userspace ELF validation
    -> MTSS PID1 admission
    -> future architecture ring-3 entry
```

## Honest status meanings

* `SPIDER-RS [FOUND]` means the loader read `/spider-rt/sbin/spider-rs`.
* `SPIDER-RS [ELF OK]` means ELF validation and Supervisor-authorized launch
  returned successfully.
* `PID1 [CREATED]` means the kernel process record exists.
* `PID1 [RUNNABLE]` means MTSS-visible PID1 task/thread admission succeeded.
* `SYSTEM DISPATCHER [PENDING: user-mode transition not implemented]` means PID1
  is runnable but the architecture backend has not entered ring 3.
* `SYSTEM DISPATCHER [STARTED]` must only be used after Spider-rs actually
  starts executing as the dispatcher.

## Remaining limitation

The current milestone creates a runnable Spider-rs PID1 record through MTSS but
does not claim user-mode execution.  Ring-3 transition support remains the next
blocker before the dispatcher can be marked started or online.
