# PID1 handoff boot order

Mirage previously attempted userspace init after the root filesystem and Supervisor were online, but before MTSS had completed its handoff. The boot log showed `userspace init launch deferred: root FS and supervisor are online; MTSS handoff not reached yet`, then `MTSS initialized`, and no retry followed.

The boot path now keeps explicit runtime dependencies: root FS, Supervisor, MTSS, Spider Runtime availability, loader start, deferred launch, PID1 creation, PID1 runnable state, and dispatcher state. `maybe_launch_pid1` refuses to launch until all required dependencies are true and records an honest deferred reason.

The critical retry point is immediately after `kernel.kernel_mtss_init()` reports `MTSS initialized` and `MTSS [Online]`. At that point the same coordinator is called again, so a launch deferred only because MTSS was offline is retried.

PID1 is located at `/spider-rt/sbin/spider-rs` through RuntimeVfs. The userspace loader reads that image, validates ELF64 x86_64 executable structure, and only then asks the Supervisor to authorize the PID1 launch. The Supervisor records policy approval and calls the kernel MTSS handoff path; it does not mutate run queues directly.

The kernel creates the process/thread through the existing MTSS-backed `spawn_task` route. PID1 is marked `Created` only after process creation succeeds and `Runnable` only after scheduler insertion succeeds.

Ring-3 dispatch is still pending. The boot status remains `Dispatcher [Pending: user-mode transition not implemented]` rather than claiming full userspace isolation is online.
