# MTSS Process and Task Lifecycle

MTSS records the portable lifecycle of Mirage tasks, processes, and micro-threads. It provides the scheduler-visible truth used by the Supervisor and boot status code, while policy decisions remain outside MTSS.

## Lifecycle objects

* **Process record:** scheduler-visible process identity, parent/child links, thread membership, address-space handle, credential/grant handles, name, path, state, and exit code.
* **Task record:** early-core scheduler object with task ID, kind (`Kernel` or `Userspace`), state, optional address-space ID, main thread, and name.
* **Thread record:** scheduler object with thread ID, owning task, state, kernel stack, optional user stack, and saved CPU context.

MTSS stores handles to address spaces, credentials, and grants. It does not own the underlying page tables, credential material, or capability policy.

## State transitions

### Userspace task creation

```text
ELF + stack + address-space preflight succeeds
    -> Task Created
    -> Thread New
    -> Task Runnable
    -> Thread Ready
    -> enqueue runnable thread
    -> schedule_next marks Thread/Task Running
```

A userspace task must not become runnable until all preflight checks succeed. Required proof includes canonical entry/stack addresses, executable entry mapping, writable stack mapping, valid user selectors, valid kernel stack/TSS state, valid address-space handle, and valid CR3.

### Blocking and sleeping

Blocked and sleeping states are MTSS-visible scheduling states. The event source may be IPC, futexes, timers, page faults, service dependencies, or supervisor containment. The wake policy and authorization come from the responsible kernel mechanism or Supervisor policy; MTSS performs the portable state transition and run-queue insertion/removal.

### Exit and reap

`exit_current()` marks the running thread as `Zombie` and the owning task as `Exited`. Full process reaping, child notification, resource reclamation, and service restart policy are supervisor/kernel integration work and must not be faked by simply changing status text.

### Fault and containment

Faults should become explicit MTSS/supervisor events. MTSS can represent that execution is no longer runnable; the Supervisor decides containment, capability revocation, and restart. A driver-service crash must not become a kernel panic unless the failed component is explicitly fatal.

## PID allocation

The early core reserves idle ID `0`. The first userspace task is expected to receive PID/task ID `1`, matching the Spider-rs PID1 handoff contract. PID1 is meaningful only after a real MTSS/kernel process record exists.

## What MTSS must not do

MTSS must not:

* authorize PID1 or services;
* grant or revoke capabilities;
* validate module signatures as policy;
* parse root filesystem policy;
* directly inspect or mutate hardware page tables except through handles and backend contracts;
* mark services online without real task/thread execution evidence.
