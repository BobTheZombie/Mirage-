# Boot Milestone 1.1

Milestone 1.1 is complete only when the real boot pipeline reaches `BOOT PROGRESS: 100%` and `CURRENT PHASE: BOOTED` with no required vague pending states.

If ring3/syscalls are incomplete, Mirage may report an honest cooperative milestone with MTSS degraded, PID1 runnable, and idleloop running, but it must not claim the terminal userspace application ran.
