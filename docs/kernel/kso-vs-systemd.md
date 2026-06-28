# KSO vs systemd

KSO and systemd-like init systems both describe ordering, dependencies, and startup status, but they operate at different layers and have different authority.

## KSO

KSO runs during Mirage kernel startup. It coordinates early kernel mechanisms and boot facts before the full userspace service environment exists. It consumes generated `no_std` tables and reports truthful component states into BootPhase.

KSO can decide that the kernel may start a built-in component, wait for a dependency, accept a degraded early boot mode, or open the PID1 handoff gate. It cannot pretend that userspace services are running, grant arbitrary service capabilities, or replace Spider runtime policy.

## systemd-like Spider runtime

`spider-rs` is Mirage PID1. `spider-rsd` is the dispatcher daemon spawned by PID1. Together they are the systemd-like runtime layer for service units, userspace dependency chains, and POSIX/GNU environment bring-up.

Spider runtime owns normal service orchestration after PID1 is really running. It launches `/usr/bin/m1-terminal` through `/etc/spider/units/default.target`, `/etc/spider/units/basic.target`, and `/etc/spider/units/m1-terminal.service`. KSO must not short-circuit that chain.

## Boundary table

| Concern | KSO | Spider runtime / Supervisor |
| --- | --- | --- |
| Kernel mechanism startup order | Owns | Observes after handoff |
| TOML source policy compilation | Consumes generated tables | May have separate userspace unit files |
| Runtime TOML parsing in kernel | Forbidden | Not applicable to KSO |
| BootPhase status source | Provides real component state | Provides later service state |
| PID0 / MTSS readiness gate | Models and gates | Uses after userspace starts |
| PID1 launch authorization | Waits for Supervisor capability | Supervisor decides authorization |
| PID1 process admission | Requires MTSS eligibility | PID1 starts after admission |
| Userspace service recovery | Not KSO | Supervisor/Spider runtime |
| Driver service restart | Not KSO | Supervisor |
| Capability grant/revoke policy | Not KSO | Supervisor |

## Why Mirage needs KSO anyway

A userspace init cannot order mechanisms that must exist before userspace can run. KSO covers this gap without turning the kernel into a monolithic init system. It keeps early boot explicit, generated, bounded, and truthful, then hands off to MTSS, Supervisor, and Spider runtime as soon as the prerequisites are real.
