# KSO boot orchestrator audit

## Scope

This audit documents the intended Kernel Startup Orchestrator architecture and the documentation updates required before implementation. It covers KSO identity, generated table policy, dependency semantics, degraded behavior, BootPhase integration, MTSS PID0 handling, PID1 handoff, and staged migration limits.

## Findings

### KSO identity

KSO is the kernel startup dependency coordinator. It owns early kernel startup order and dependency policy. It is not a userspace init system, not Spider-rs, not Spider-rsd, and not a Supervisor replacement.

### TOML source-only rule

KSO policy TOML is acceptable as a source format because it is reviewable and schema-validatable. The target kernel must consume generated `no_std` tables only. TOML parsing must not be linked into the early boot path or target image.

### Dependency semantics

KSO must distinguish:

* `requires` — hard startup dependency;
* `wants` — preferred soft dependency;
* `after` — ordering-only edge;
* `before` — reverse ordering hint;
* `provides` — capability or boot fact published after real success;
* `conflicts` — mutually exclusive node or capability.

A dependency graph that collapses these into a single ordering list would violate the KSO contract.

### Required, optional, and degraded nodes

Required nodes protect boot invariants and fail fatal unless policy defines a real degraded capability. Optional nodes must not block required boot. Degraded status must name the reduced capability set and must not be reported as online.

### Retry behavior

Waiting nodes must be retried after relevant graph changes: new capabilities, degraded outcomes, optional skips, MTSS readiness updates, Supervisor readiness, RuntimeVfs/rootfs availability, and loader readiness. Retry must be progress-triggered and bounded rather than a busy loop.

### BootPhase and live UI

BootPhase should expose KSO state to renderers. The live milestone UI reads BootPhase/KSO state and must not drive boot progress, hardware probing, dependency satisfaction, or status success.

### MTSS PID0 and PID1 handoff

MTSS is PID0. KSO should model MTSS capabilities at several levels: core, scheduler admission, cooperative degraded mode, timer delivery, preemption, and full online. PID1 handoff must be policy-driven and may use cooperative MTSS only when `require_preemption_for_userspace = false`.

PID1 handoff requires real RuntimeVfs/rootfs access, required Spider runtime files, Supervisor authorization, userspace loader preflight, MTSS admission eligibility, and architecture entry readiness. KSO must not mark PID1, Spider-rs, Spider-rsd, or M1 Terminal as running.

## Staged migration constraints

The following phases may remain linear during staged migration:

1. bootloader handoff;
2. earliest CPU entry and stack setup;
3. minimal serial/BOOTDIAG availability;
4. initial memory map capture;
5. initial framebuffer discovery;
6. first BootPhase construction;
7. final architecture ring-3 entry sequence after PID1 admission.

All other policy-like startup ordering should migrate toward KSO nodes.

## Recommended implementation checklist

1. Define the TOML schema and validation tool.
2. Generate bounded `no_std` tables from TOML.
3. Add KSO node state tracking.
4. Add dependency evaluation for all six edge types.
5. Add required/optional/degraded failure handling.
6. Add progress-triggered retry.
7. Wire KSO state into BootPhase.
8. Model MTSS PID0 capabilities separately from full MTSS online.
9. Drive PID1 handoff from KSO policy facts.
10. Add positive and negative graph validation tests.
