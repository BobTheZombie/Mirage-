# Kernel Startup Orchestrator (KSO)

The Kernel Startup Orchestrator (KSO) is Mirage's kernel-side boot dependency coordinator. It owns the ordered admission of early kernel components into the boot graph and records the real status each component reports while the system moves from firmware handoff toward MTSS PID0 and PID1 eligibility.

KSO exists to make startup policy explicit instead of spreading boot ordering across ad-hoc probe branches. Its job is to answer these questions during early boot:

* Which kernel component may start now?
* Which dependencies are required and which are only preferred?
* Which capabilities or boot services has a component provided?
* Which components are waiting for dependencies that may appear later?
* Which failures are fatal and which failures permit degraded boot?
* Which BootPhase state should be reported to the live milestone UI?

## What KSO is

KSO is:

* a dependency graph evaluator for kernel startup nodes;
* the owner of kernel startup order and dependency policy;
* a consumer of generated `no_std` startup tables;
* the source of truth for component startup state during early boot;
* the bridge from component reports to `BootPhase` status;
* the policy gate that decides when PID1 handoff dependencies are satisfied;
* the mechanism that retries waiting nodes when newly provided capabilities unblock them.

KSO is part of the kernel boot path because kernel mechanisms must be initialized before the Supervisor and userspace services can take over higher-level policy. KSO still preserves Mirage boundaries: it orders and records early kernel startup, but it does not grant arbitrary service authority or fake component success.

## What KSO is not

KSO is not:

* a userspace init system;
* a replacement for `spider-rs` PID1 or `spider-rsd`;
* a service supervisor for normal runtime services;
* a driver crash-recovery manager;
* a capability policy broker;
* a TOML parser in the target kernel;
* a reason to move Supervisor policy into the kernel;
* a source of synthetic `OK`, `ONLINE`, `RUNNING`, `RUNNABLE`, or `BOOTED` statuses.

KSO stops at early startup dependency policy. Once the Supervisor, MTSS, RuntimeVfs/rootfs, and PID1 handoff prerequisites are real, policy for service lifecycle, recovery, launch authorization, and capability granting belongs to the Supervisor and Spider runtime stack.

## Source TOML and generated kernel tables

KSO policies are authored as TOML because TOML is reviewable, diff-friendly, schema-validatable, and suitable for documenting why a boot dependency exists. TOML is source-only. The target kernel must never parse TOML during early boot.

Instead, build tooling validates KSO TOML and generates compact `no_std` Rust tables or another reviewed compact target format. The kernel consumes only those generated tables. This keeps early boot deterministic and avoids pulling TOML parsing, allocation-heavy data structures, stringly policy interpretation, or host tooling assumptions into the kernel image.

The generated table contract is:

* every node has a stable identifier;
* dependency arrays are bounded and generated ahead of time;
* string metadata is either omitted from the target image or stored in bounded static data;
* unknown dependency kinds fail validation before boot;
* target lookup is table-based and does not require parsing source files;
* regenerated tables are reviewed together with their TOML source diff.

## Runtime states

KSO tracks each node through explicit states:

* `Pending` — node exists but has not yet been considered ready to start.
* `Waiting` — node is blocked by unmet dependencies, conflicts, or policy.
* `Starting` — KSO has called the component start hook or admitted the component to startup.
* `Provided` — node completed enough to publish its declared `provides` capabilities.
* `Online` — node is fully initialized and truthfully online.
* `Degraded` — node initialized partially, but policy allows boot to continue.
* `Skipped` — optional node was intentionally not started on this platform or configuration.
* `Failed` — node failed with a typed reason.
* `Fatal` — required node failure or conflict makes boot continuation invalid.

A node may publish a `provides` capability only after the backing component actually reached the state promised by that capability. A database match, probe attempt, or optimistic assumption is not enough.

## Required and optional nodes

A required node protects a boot invariant. If it fails, KSO must either stop boot with a fatal failure or enter an explicitly declared degraded mode. Required nodes are used for critical mechanisms such as memory setup, architecture state, interrupt entry, BootPhase reporting, supervisor construction prerequisites, MTSS PID0 prerequisites, RuntimeVfs/rootfs prerequisites, and PID1 handoff gates.

An optional node improves platform support but must not block required boot unless a policy explicitly marks it fatal for that build. Optional driver policies are used for device classes such as USB, input, NVMe, AHCI variants, graphics acceleration, and platform telemetry. Optional failure should produce a precise degraded or disabled status, not a panic or infinite wait.

## Degraded handling

Degraded boot is a first-class KSO result. It means a node or dependency did not reach full online status, but policy permits a narrower capability set. Examples include cooperative MTSS readiness before timer-backed preemption, framebuffer fallback to serial BOOTDIAG, or an optional input driver being disabled while boot continues.

Degraded status must name the missing capability. For example, `mtss.scheduler.cooperative` may be enough for PID1 admission when policy permits cooperative userspace, while `mtss.scheduler.preemptive` remains pending. Do not collapse degraded into online.

## Retry behavior

KSO must retry waiting nodes whenever the graph changes. Triggers include:

* a node publishing a new `provides` capability;
* a node failing or degrading in a way that changes optional dependency handling;
* MTSS readiness changing from core to scheduler-ready to degraded to preemptive;
* Supervisor construction becoming available;
* RuntimeVfs/rootfs becoming available;
* a conflict being resolved by skipping or disabling an optional node.

Retry is bounded by graph progress. KSO should not busy-loop on a permanently waiting node; it records the reason and re-evaluates only when a relevant status changes.

## BootPhase integration

BootPhase is the observable boot state. KSO owns the component state that feeds BootPhase, while the live milestone UI only renders BootPhase. The UI must not drive KSO, unblock dependencies, poll hardware indefinitely, or mark a node successful.

KSO reports BootPhase transitions when real state changes occur: node admitted, dependency satisfied, provided capability published, degraded result accepted, fatal failure recorded, or PID1 handoff gate opened. Serial diagnostics may include more detail, but framebuffer status remains concise and truthful.

## MTSS PID0 policy

MTSS is PID0, the kernel execution root for Mirage scheduling mechanics. KSO policy should model MTSS as a provider of scheduler capabilities rather than as a normal userspace service. At minimum, policies should distinguish:

* MTSS core data structures initialized;
* scheduler admission APIs ready;
* idle fallback available;
* cooperative scheduling degraded mode available;
* timer delivery ready;
* preemption ready;
* full MTSS online.

KSO must not require full `MTSS ONLINE` for PID1 unless the selected policy explicitly sets `require_preemption_for_userspace = true`.

## PID1 handoff policy

PID1 handoff is dependency-policy driven. KSO should open the handoff gate only when all required prerequisites are real:

* RuntimeVfs/rootfs can read `/spider-rt/sbin/spider-rs`;
* `/spider-rt/sbin/spider-rsd` is present in the runtime image;
* required unit files for the Spider runtime path are present;
* Supervisor launch authorization is available;
* userspace loader preflight can validate the PID1 ELF and stack;
* MTSS has an eligible admission mode;
* architecture entry code can truthfully report whether user execution has begun.

PID1 may be `CREATED` only after real process records exist, `RUNNABLE` only after MTSS admission, and `RUNNING` only after actual user-mode execution begins.

## Adding a kernel component

To add a kernel component to KSO:

1. Define the component's stable node identifier.
2. List exact capabilities it requires before start.
3. List soft dependencies as `wants`, never as `requires`, unless boot is invalid without them.
4. List ordering-only edges with `after` or `before`.
5. Declare the capabilities it truthfully `provides` after successful initialization.
6. Declare conflicts that cannot run in the same boot profile.
7. Choose `required` or `optional` and document degraded behavior.
8. Add the TOML policy entry.
9. Regenerate the `no_std` table.
10. Add validation evidence for the schema and for positive/negative dependency behavior.

## Adding an optional driver policy

Optional driver policy must be bounded and honest:

1. Mark the node `optional` unless the platform profile truly cannot boot without it.
2. Use `wants` for preferred facilities such as PCI enumeration, IRQ routing, DMA windows, or device database hints.
3. Use `requires` only for mechanisms without which probing would be unsafe.
4. Make probe loops bounded and declare failure as `Degraded`, `Skipped`, or `Failed` with a reason.
5. Do not publish an `*.online` capability until real hardware initialization succeeds.
6. Keep device ownership, service launch, and capability grants in Supervisor policy.

## Linear phases during staged migration

KSO is introduced gradually. During migration, these phases may remain linear even while individual later phases move into the dependency graph:

1. bootloader handoff;
2. earliest architecture entry and stack setup;
3. minimal serial/BOOTDIAG availability;
4. initial memory map capture needed to allocate KSO state;
5. initial framebuffer discovery needed for live milestone rendering;
6. first BootPhase construction;
7. final architecture ring-3 entry sequence after PID1 is admitted.

These linear phases should shrink over time, but they must remain truthful. A linear phase is acceptable only when it is an unavoidable mechanism boundary, not a hidden policy shortcut.
