# KSO policy schema

KSO policy is authored in TOML and compiled into generated `no_std` kernel tables before boot. The TOML files are source artifacts for humans, review tools, schema validators, and table generators. They are not target-kernel configuration files.

## Top-level structure

A KSO policy contains metadata and one or more startup nodes:

```toml
schema_version = 1
policy_id = "mirage.default"
require_preemption_for_userspace = false

[[node]]
id = "mtss"
kind = "kernel_component"
required = true
requires = ["arch.context", "memory.vm"]
wants = ["timer.tick"]
after = ["interrupts"]
before = ["pid1.handoff"]
provides = ["mtss.core", "mtss.scheduler.cooperative"]
conflicts = []
degraded_provides = ["mtss.scheduler.cooperative"]
on_failure = "fatal"
```

## Fields

### `schema_version`

Integer schema version. Generators must reject unsupported versions.

### `policy_id`

Stable policy name used in diagnostics and generated table provenance.

### `require_preemption_for_userspace`

Boolean PID1 handoff switch. When `false`, cooperative MTSS readiness may satisfy PID1 admission if all other prerequisites are real. When `true`, PID1 waits for preemptive MTSS readiness.

### `node.id`

Stable node identifier. Identifiers should be short, namespaced where useful, and stable across generated tables.

### `node.kind`

Classifier for diagnostics and validation. Suggested values:

* `arch_component`
* `kernel_component`
* `driver_builtin`
* `driver_optional`
* `boot_service`
* `handoff_gate`

### `node.required`

Boolean required/optional flag. Required node failure is fatal unless `on_failure` explicitly allows a degraded state. Optional node failure must not block required boot.

### `node.requires`

Hard dependencies. Every listed capability or node must be satisfied before startup. Missing `requires` keeps the node waiting. Failed required dependencies are fatal unless policy defines degraded handling.

### `node.wants`

Soft dependencies. KSO should try to start the node after wanted capabilities are present, but missing wants do not by themselves block startup forever. Wants are appropriate for optional acceleration, optional diagnostics, or preferred platform features.

### `node.after`

Ordering-only dependencies. `after` means this node should be considered after another node or phase, but it does not imply a capability requirement. Use `after` when the relationship is sequencing rather than authority or resource availability.

### `node.before`

Inverse ordering hint. `before` says this node should be ordered before another node or gate. Generators may normalize `before` into corresponding `after` edges.

### `node.provides`

Capabilities or boot facts published after real successful initialization. A node must not publish a capability merely because it matched policy, began probing, or is expected to work.

### `node.conflicts`

Mutually exclusive nodes or capabilities. Conflicts are used when two implementations cannot safely coexist in the same boot profile.

### `node.degraded_provides`

Capabilities available in a degraded state. These must be narrower than full `provides` and must identify what remains usable.

### `node.on_failure`

Failure policy. Suggested values:

* `fatal` — stop required boot.
* `degrade` — continue with `degraded_provides` if validation accepts them.
* `skip` — skip optional node.
* `disable_device` — mark optional hardware disabled and continue.

## Dependency semantics

| Edge | Blocks start? | Publishes capability? | Intended use |
| --- | --- | --- | --- |
| `requires` | Yes | No | Mandatory capabilities or boot facts. |
| `wants` | No, after bounded waiting | No | Preferred optional facilities. |
| `after` | Ordering only | No | Sequencing without authority. |
| `before` | Ordering only | No | Reverse sequencing hint. |
| `provides` | N/A | Yes, after real success | Capabilities made available. |
| `conflicts` | Yes, if active together | No | Mutually exclusive nodes. |

## Required and optional examples

Required memory setup:

```toml
[[node]]
id = "memory.vm"
kind = "kernel_component"
required = true
requires = ["arch.bootstrap"]
provides = ["memory.vm", "memory.alloc"]
on_failure = "fatal"
```

Optional USB input:

```toml
[[node]]
id = "driver.usb.input"
kind = "driver_optional"
required = false
requires = ["pci.enumerated", "memory.dma.safe"]
wants = ["interrupts.msi", "device_db.usb"]
after = ["driver.xhci"]
provides = ["input.usb.online"]
on_failure = "disable_device"
```

## Validation requirements

Policy generators must validate:

* duplicate node identifiers;
* unsupported schema versions;
* unknown edge kinds;
* unresolved hard dependencies;
* cycles that cannot be reduced to ordering-only constraints;
* required nodes with non-fatal failure policy but no degraded capability;
* optional nodes that publish online capabilities before a real probe result;
* PID1 handoff gates that omit RuntimeVfs, Supervisor authorization, userspace loader, Spider runtime, and MTSS admission prerequisites.
