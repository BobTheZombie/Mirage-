# AMD Timers

This document describes the Mirage position on AMD and AMD64 timer support. Timers are kernel mechanisms when they drive scheduling and interrupt delivery; policy such as service timeouts and restart delays belongs in the supervisor.

## Implemented now

* Mirage treats timer hardware as an early machine-control mechanism rather than a high-level policy service.
* The AMD64 layer is positioned to hold low-level CPU mechanism validation and instruction boundaries.
* Platform documentation recognizes that the kernel may need a minimal boot timer for scheduling and interrupt calibration.
* Supervisor-owned services can express restart policy independently of timer hardware details.

## Mocked now

* There is no complete real-hardware AMD timer driver in the current skeleton.
* Timer calibration, frequency discovery, APIC timer configuration, HPET selection, and invariant TSC validation are represented as future hardware bring-up work.
* Service timeouts and restart delays are policy concepts, but they are not yet backed by a production monotonic clock service.
* Tests can model timer decisions without touching APIC, HPET, PM timer, or model-specific registers.

## Real hardware path

* Discover architectural timer capabilities through CPUID feature bits and firmware tables.
* Prefer stable architectural mechanisms for monotonic time and scheduler ticks, such as invariant TSC when validated for the platform.
* Use local APIC timer or equivalent interrupt sources for per-CPU scheduling once interrupt routing is initialized.
* Treat HPET, ACPI PM timer, and platform timers as selectable clock sources only after their MMIO/IO regions are covered by capabilities.
* Keep timer interrupt handling in kernel mechanism code, but expose time services to the supervisor and user-facing runtime through IPC and capability-controlled interfaces.
* Log the selected clock source, calibration source, frequency, and fallback path in the boot platform report.

## Unsupported areas

* Mirage does not currently support production-grade suspend/resume timekeeping, deep C-state timer compensation, or cross-socket clock skew correction.
* Vendor-private timer registers and undocumented chipset timer paths are unsupported.
* Unvalidated TSC behavior is not acceptable as the sole monotonic source on real hardware.
* User applications must not receive raw timer MMIO or model-specific register access.
* The kernel must not grow policy such as service restart strategy, watchdog escalation policy, or wall-clock time synchronization.

## Next steps

* Add a clock-source trait that separates timer mechanism operations from supervisor policy.
* Add CPUID-based invariant TSC detection and a mock calibration path.
* Add APIC timer descriptors with explicit IRQ/vector ownership.
* Add a supervisor-visible monotonic time service contract.
* Add tests for timer fallback order, capability checks, and unsupported-clock rejection.
