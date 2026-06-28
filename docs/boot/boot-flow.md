# Mirage boot flow

The required boot continuation pipeline is moving from a mostly linear sequence to KSO-managed dependency policy. During the staged migration, the boot flow remains truthful: linear phases are mechanism boundaries, and graph-managed phases are reported by KSO through BootPhase.

## Current high-level pipeline

1. early architecture initialization
2. memory and framebuffer initialization
3. interrupt and platform probe
4. optional storage and input probe
5. kernel construction
6. boot-info application
7. KSO table availability and startup graph construction
8. supervisor construction
9. boot runtime validation and RuntimeVfs mount
10. root filesystem mount
11. supervisor service initialization
12. MTSS/PID0 initialization
13. PID1 handoff eligibility
14. userspace loader start
15. spider-rs ELF load and preflight
16. PID1 process/thread creation and MTSS runnable admission
17. scheduler/idleloop entry
18. architecture user-entry attempt when all preflight checks pass

The boot UI reflects this state only. It must not drive boot progress or block continuation.

## KSO-managed continuation

KSO owns startup order and dependency policy after the earliest architecture and diagnostic mechanisms are available. KSO consumes generated `no_std` tables, evaluates `requires`, `wants`, `after`, `before`, `provides`, and `conflicts`, then updates BootPhase as real component results arrive.

A component may report `Online` only after the underlying code path has completed. Optional device failure may produce `Degraded`, `Skipped`, or `Failed` status without blocking required boot. Required failure is fatal unless policy explicitly names a degraded capability that remains valid.

## Linear phases during migration

The following phases remain linear while KSO is introduced:

* bootloader handoff;
* earliest CPU entry, stack setup, and architecture invariants;
* minimal serial/BOOTDIAG setup;
* initial memory map capture needed to allocate boot state;
* initial framebuffer discovery for the milestone renderer;
* first BootPhase object construction;
* final architecture ring-3 entry sequence after PID1 is admitted.

These phases must not become hidden policy shortcuts. Any phase that can be represented as a dependency node should move into KSO over time.

## PID1 path

PID1 handoff is opened by policy, not by hardcoded `MTSS ONLINE` checks. The handoff gate requires real RuntimeVfs/rootfs access, Spider runtime file validation, Supervisor launch authorization, userspace loader preflight, an eligible MTSS PID0 state, and architecture entry readiness. Cooperative MTSS may be enough when `require_preemption_for_userspace = false`; full preemption is required when that policy switch is true.
