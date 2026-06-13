# Mirage-dispatch-rs

Mirage-dispatch-rs is the kernel component startup dispatcher. It owns registration, dependency ordering, and startup dispatch for kernel components only. It does not own userspace service policy, Spider-rs lifecycle policy, recovery policy, or PID 1 behavior.
