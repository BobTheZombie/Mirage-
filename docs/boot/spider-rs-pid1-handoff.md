# spider-rs PID1 handoff

Boot order is Kernel -> MTSS -> Userspace Loader -> `/spider-rt/sbin/spider-rs` as PID1 -> `/spider-rt/sbin/spider-rsd` -> `/usr/bin/m1-terminal`.

The implemented kernel path starts the userspace loader, reads the Spider Runtime image, validates the Spider ELF, creates PID1, inserts the MTSS task, and reports `SPIDER-RS PID1 [RUNNABLE]` only after admission succeeds. Because the architecture ring-3 transition remains incomplete, the dispatcher is reported as pending instead of online.
