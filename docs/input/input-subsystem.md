# Mirage input subsystem

The early input subsystem is a fixed-size, no-heap event queue for decoded input
mechanisms.  Architecture drivers publish `KeyboardEvent` values containing a
source, physical key code, key state, modifiers, raw code, and optional ASCII.

The queue is bounded.  Normal producers may take the queue lock briefly; IRQ
producers use a non-blocking publish path and increment drop counters if the
queue is busy or full.  Overflow never panics and never blocks boot.

Consumers include the debug shell today and future supervised userspace input
services later.  The supervisor observes input facts and approves routing, but it
does not own i8042 port I/O.
