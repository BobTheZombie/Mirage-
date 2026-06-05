# AMD Timers

This document describes the Mirage position on AMD and AMD64 timer support.
Timers are kernel/platform mechanisms when they drive scheduling, calibration,
and interrupt delivery. Policy such as service timeouts, restart delays,
watchdog escalation, wall-clock sync, and power-management strategy belongs in
the supervisor or supervised services.

## Implemented now

* `mirage-platform` exposes mechanism-only timer abstractions:
  `PlatformTimer`, `TscTimer`, `ApicTimer`, `HpetTimer`, and
  `PitFallbackTimer`.
* `calibrate_timer()` selects a clock source from explicit discovery facts using
  this priority:

```text
invariant TSC if valid
    -> APIC timer
    -> HPET if discovered
    -> PIT fallback
```

* `monotonic_now()` and `timer_frequency()` expose a selected timer's monotonic
  nanosecond counter and calibrated frequency without implying scheduler policy.
* Invariant TSC selection depends on `mirage-amd64` CPUID feature data. Mirage
  requires the architectural TSC bit and AMD invariant-TSC bit before selecting
  `TscTimer`.
* Ryzen platform descriptors from `mirage-ryzen` are consumed as structured
  mechanism facts. Ryzen quirks can require explicit invariant-TSC validation;
  they do not grant authority or decide supervisor policy.
* Mock calibration supports deterministic tests without touching APIC MMIO, HPET
  MMIO, PIT I/O ports, MSRs, or host hardware timers.

## Ownership boundaries

| Layer | Timer responsibility |
| --- | --- |
| AMD64 mechanism crate | CPUID feature parsing, instruction/MSR boundaries, low-level facts. |
| Ryzen mechanism crate | Ryzen generation/profile/quirk descriptors, not policy decisions. |
| Platform mechanism crate | Timer selection, calibration math, monotonic/frequency primitives. |
| Kernel | Interrupt delivery, timer tick dispatch, capability enforcement for timer/IRQ resources. |
| Supervisor | Service timeout policy, watchdog policy, restart delays, capability grant/revoke decisions. |
| Driver services | Use granted timer/IRQ capabilities; no raw timer authority by default. |
| Applications | Observe POSIX/GNU-compatible time APIs through libc/runtime surfaces. |

The platform timer code must remain a mechanism selector. It must not decide:

* which service is restarted after a timeout;
* how long a driver should be allowed to recover;
* whether a watchdog escalates to reboot;
* wall-clock/NTP policy;
* user-visible POSIX clock semantics beyond providing monotonic primitives.

## Timer source notes

### `TscTimer`

`TscTimer` is preferred only when CPUID reports TSC and invariant TSC and Ryzen
quirk checks do not invalidate that requirement. Calibration converts TSC deltas
against a reference interval into Hz. Polling calibration has an explicit poll
limit and returns `CalibrationTimeout` when the reference timer fails to advance.

### `ApicTimer`

`ApicTimer` is second priority and represents a local APIC timer whose frequency
has already been discovered or calibrated by early platform code. It is a timer
mechanism; APIC vector routing and interrupt ownership remain kernel-owned.

### `HpetTimer`

`HpetTimer` is selected only when HPET has been discovered and a non-zero
frequency is supplied. HPET MMIO ownership must be capability-covered before any
real hardware access is implemented.

### `PitFallbackTimer`

`PitFallbackTimer` is the last-resort legacy mechanism. It exists to keep early
bring-up explicit, not to make PIT a preferred production timer.

## Mocked / not complete yet

* There is still no production real-hardware AMD timer driver in this skeleton.
* APIC, HPET, PIT, and TSC reads are not performed by `mirage-platform`; callers
  inject discovery and calibration facts.
* Suspend/resume timekeeping, deep C-state compensation, cross-socket skew
  correction, deadline timers, and wall-clock synchronization are planned work.
* Timer IRQ delivery to services through capability-checked IPC is not complete.

## Real hardware path

* Discover architectural timer capabilities through CPUID feature bits and
  firmware tables.
* Validate invariant TSC before relying on it as the monotonic source.
* Use local APIC timer or equivalent interrupt sources for per-CPU scheduling
  once interrupt routing is initialized.
* Treat HPET, ACPI PM timer, and legacy PIT/PM paths as selectable mechanisms
  only after their MMIO/IO regions are covered by capabilities.
* Keep timer interrupt handling in kernel mechanism code, but expose time
  services to the supervisor and user-facing runtime through IPC and
  capability-controlled interfaces.
* Log selected source, calibration source, frequency, and fallback path in the
  boot platform report.

## Tests

Current timer tests cover:

* mock TSC calibration math;
* fallback order from TSC to APIC to HPET to PIT;
* invalid/non-invariant TSC fallback;
* timeout behavior for polling calibration loops.
