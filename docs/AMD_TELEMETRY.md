# AMD Telemetry Service

AMD telemetry in Mirage is a supervised service concern, not a reason to expose raw machine registers to applications. The initial goal is safe, auditable reporting of platform facts and sensor-like channels where public references make the path supportable.

## Implemented now

* Mirage has named Ryzen telemetry channel identifiers for temperature, package power, and core voltage.
* Mirage has an AMD telemetry platform service kind.
* The platform planner can create an AMD telemetry service launch request.
* The telemetry service is marked for manual recovery rather than automatic restart-on-crash in the current platform policy.
* Telemetry is modeled as mechanism data plus supervisor policy rather than application-owned hardware access.

## Mocked now

* Telemetry channels are descriptors only; they do not read real sensor registers.
* No production `amd-telemetryd` service exists yet.
* Sensor scaling, update cadence, per-family register selection, and fault handling are not implemented.
* PPR-specific telemetry availability is not encoded as a complete rule table.
* There is no user-facing metrics ABI beyond the general direction that service APIs should use IPC and capabilities.

## Real hardware path

* Gate telemetry support on CPUID vendor, family, model, stepping, feature bits, and PPR availability.
* Discover any telemetry-related PCI or MMIO resources through normal platform discovery before granting access.
* Run telemetry collection in a supervised service with read-only or narrowly scoped IO capabilities whenever possible.
* Normalize readings in the service and expose them through an IPC API that returns values, units, freshness, and error state.
* Let the supervisor decide which consumers can subscribe to telemetry data and which channels are visible.
* Treat telemetry failures as service-level faults, not kernel panics, unless they expose a deeper machine-check or interrupt failure.

## Unsupported areas

* Undocumented sensor registers and vendor-private telemetry paths are unsupported.
* Voltage tuning, overclocking controls, fan curves, and power-limit mutation are unsupported in the initial telemetry service.
* Applications must not receive raw MSR, SMN, PCI config, or MMIO access for telemetry.
* Telemetry availability must not be inferred from Ryzen branding alone.
* Safety-critical thermal management is not delegated to an experimental telemetry daemon in early versions.

## Next steps

* Add an `amd-telemetryd` service manifest and IPC schema.
* Add per-channel metadata for units, expected update cadence, and error reporting.
* Add PPR-gated availability records per CPU family/model/stepping.
* Add mock telemetry provider tests with supervisor capability checks.
* Add boot-report telemetry discovery status so unsupported channels are explicit.
