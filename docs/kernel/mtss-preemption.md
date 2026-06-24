# MTSS Preemption, Timer Source, and Context Switching

This document describes the current preemption and context-switching contract between MTSS, the Mirage kernel, and the x86_64 backend.

## Ownership split

* MTSS owns scheduler-visible time-slice accounting, run-queue rotation, and the decision to select another runnable thread.
* The architecture backend owns interrupt entry, trap-frame layout, CR3 switching, TSS/RSP0 setup, and `iretq` restoration.
* The kernel glues the two layers together by delivering timer ticks to scheduler code without moving portable scheduling policy into interrupt handlers.

## Timer source

The current x86_64 early timer source is the legacy PIC/PIT timer interrupt vector (`32`). The IDT dispatcher increments the architecture timer tick counter when the timer vector is observed and acknowledges the IRQ. MTSS consumes this as a scheduling event through `on_timer_tick()`/`schedule_next()` rather than programming the hardware timer itself.

Future timer backends may use APIC, HPET, TSC deadline, or platform-specific timers. They must still expose monotonic scheduler time and bounded preemption delivery without giving MTSS direct hardware policy ownership.

## Preemption path

The intended hardware path is:

```text
hardware timer IRQ
    -> x86_64 interrupt stub saves CpuContext
    -> IDT records timer tick and EOI
    -> kernel scheduler/tick hook calls MTSS on_timer_tick()
    -> MTSS rotates runnable state and selects next thread
    -> x86_64 backend restores selected CpuContext
    -> iretq returns to kernel or user context
```

The current `run_thread_slice` path detects timer preemption by either observing a saved timer trap vector or by seeing the architecture timer tick counter change during the slice. Syscall traps are reported separately so syscall dispatch does not masquerade as timer preemption.

## Context switching

The architecture context contract uses a single saved `CpuContext` layout for assembly and Rust. It contains general registers, `rip`, `cs`, `rflags`, `rsp`, `ss`, segment bases, trap vector, error code, and privilege mode. The x86_64 backend publishes the current core/thread/context pointer before restore so trap entry can save the return frame into the running thread's context.

Before restoring a userspace frame, the backend sanitizes the return frame:

* user `rip` and `rsp` must be canonical user addresses;
* user code/data selectors must match the Mirage user selectors;
* RFLAGS must retain interrupt/reserved bits and must not preserve trap flag;
* kernel stack/TSS state must be prepared for the CPU.

## Scheduler behavior on tick

`CoreMtss::schedule_next()`:

1. returns the current running non-idle thread to `Ready` and re-enqueues it;
2. dequeues the next non-idle ready thread, skipping idle;
3. selects idle only if no non-idle runnable thread exists;
4. marks the selected thread `Running` and its task `Running`;
5. stores the selected thread as current.

This is deliberately simple FIFO round-robin behavior for the early architecture skeleton.

## Safety constraints

* Timer interrupt handling must be bounded; no infinite polling loops.
* Preemption must not claim a thread entered user mode unless the architecture restore path actually executed.
* MTSS must not program raw hardware timers directly.
* The kernel/architecture backend must not create scheduler-visible policy that bypasses MTSS.
