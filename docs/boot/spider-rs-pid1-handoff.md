# Spider-rs PID1 handoff

The initial userspace handoff is staged and intentionally does not fake ring-3 success.

1. `USERSPACE LOADER [STARTED]` begins the RuntimeVfs lookup.
2. `SPIDER-RS [FOUND]` means `/spider-rt/sbin/spider-rs` was read from the immutable boot runtime image.
3. `SPIDER-RS [ELF OK]` means the kernel userspace loader validated ELF magic, class, endianness, x86_64 machine type, executable type, program headers, load segments, and that the entry point falls inside a load segment.
4. `PID1 [CREATED]` means the supervisor authorized the launch and the kernel created the process record through the MTSS-backed spawn path.
5. `PID1 [RUNNABLE]` means the task is admitted to MTSS. `DISPATCHER [PENDING]` remains until the architecture backend confirms a real userspace transition or first syscall.

The supervisor makes the launch authorization decision. The kernel performs ELF validation and process creation. MTSS owns runnable state. The supervisor does not mutate MTSS queues directly.
