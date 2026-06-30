# Ring3 Transition

Current status: pending.

The runnable PID1 milestone does not prove user-mode execution. To claim running, Mirage must validate the ELF64 image, map each PT_LOAD segment, allocate and map a writable aligned user stack, prepare a valid initial trap frame with user CS/SS selectors, enter user mode via the architecture transition path, and observe a first userspace action such as write/yield/exit.

Until then the honest status is `SPIDER-RS PID1 [PENDING: ring3 transition not implemented]`.
