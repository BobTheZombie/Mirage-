# spider-rs PID1

`/spider-rt/sbin/spider-rs` is the Mirage PID1 bootstrap process. The kernel locates it in immutable Spider Runtime, validates its ELF image, creates PID 1, and admits the task to MTSS. The current kernel reaches the honest runnable milestone; x86_64 ring-3 entry and syscall dispatch are still the blocker for executing the PID1 loop.

When executed, PID1 initializes the Mirage userspace syscall shim, spawns `/spider-rt/sbin/spider-rsd`, waits for it, and applies a bounded restart policy. It does not manage kernel internals directly.
