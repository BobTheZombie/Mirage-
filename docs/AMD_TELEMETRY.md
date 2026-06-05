# AMD Telemetry Scaffold

Mirage AMD telemetry is a **scaffold only**. It exists to model read-only AMD platform telemetry data for future supervised services without granting applications raw hardware authority.

## Scope

The scaffold provides:

* `AmdThermalSensor` for read-only thermal samples.
* `AmdPowerState` for read-only mock power-state samples.
* `AmdPstateInfo` for structured AMD P-state support discovery.
* `AmdTelemetry` for combining mock telemetry facts into one snapshot.
* `AmdTelemetryError` for explicit unsupported, unavailable, and gated-path failures.
* `read_temperature_mock()` for deterministic mock temperature data.
* `read_power_state_mock()` for deterministic mock power-state data.
* `detect_pstate_support()` for conservative support classification from already-discovered Ryzen mechanism facts.

These APIs are intended for platform-service scaffolding, supervisor policy experiments, and tests. They are not a production sensor driver.

## Safety boundary

This telemetry work is explicitly **read-only** and **non-tuning**.

Mirage AMD telemetry does **not** implement:

* undervolting;
* overclocking;
* voltage or frequency tuning;
* boost control;
* fan-curve control;
* power-limit mutation;
* firmware poking;
* permanent firmware changes;
* unsafe SMU writes;
* application access to raw MSR, SMN, PCI config, or MMIO resources.

Any future real AMD telemetry path must be gated behind the `hw-amd-telemetry` feature and must remain supervised, capability-scoped, auditable, and read-only unless a later architecture document explicitly authorizes a different mechanism.

## Mirage architecture fit

Telemetry belongs in a supervised platform service, not as a monolithic kernel policy feature. The kernel should only enforce capabilities and provide low-level mechanisms. The supervisor decides whether a telemetry service may start, which consumers may subscribe, and which read-only channels are visible.

Telemetry failures should be represented as service-level errors and recovery events. A telemetry read failure must not become a kernel panic unless it reveals a deeper machine-check, interrupt, or privilege-boundary failure.

## Current limitations

* All sensor values are mock values.
* P-state support detection is conservative and generation-bucketed.
* There is no production `amd-telemetryd` service yet.
* There is no metrics IPC ABI yet.
* Family/model/stepping-specific PPR rule tables are not complete.
