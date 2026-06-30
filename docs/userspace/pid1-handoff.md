# PID1 Handoff

PID1 handoff requires:

- boot runtime mounted;
- `/spider-rt/sbin/spider-rs` found;
- rootfs online;
- supervisor online and launch grants available;
- userspace loader started;
- MTSS core and scheduler ready.

`SPIDER-RS PID1 [CREATED]` means a real process object exists. `SPIDER-RS PID1 [RUNNABLE]` means MTSS accepted the task/thread into a run queue. `SPIDER-RS PID1 [RUNNING]` is reserved for real user code execution and must not be printed by kernel-side simulation.
