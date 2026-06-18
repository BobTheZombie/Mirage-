# Spider-rs System Dispatcher

`spider-rs` is Mirage PID1 and the userspace system dispatcher daemon. The supervisor authorizes its launch, the userspace loader validates its ELF image, and MTSS owns the runnable task admission.

Spider-rs is responsible for becoming the service root and, in a later milestone, launching child userspace applications. The kernel must not call Spider-rs as a Rust function and must not print child application output while claiming it came from userspace.

## Current state

Implemented:

- RuntimeVfs lookup of `/spider-rt/sbin/spider-rs`
- ELF validation before any PID1 status is claimed
- PID1 process record creation
- MTSS userspace task creation and runnable admission
- supervisor-owned PID1 launch report with path, process ID, task ID, thread ID, and dispatcher status

Pending:

- ring-3 transition into the Spider-rs entry point
- dispatcher main loop becoming online
- child app spawning
- console syscall/service ABI for child output
