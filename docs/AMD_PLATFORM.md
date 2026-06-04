# AMD Platform Support

This document tracks the Mirage AMD platform plan at the architecture boundary between low-level AMD64/Ryzen mechanisms and supervisor-owned policy. The goal is to support AMD systems without turning the Mirage kernel into a Linux-style hardware-policy monolith.

## Implemented now

* Mirage has an AMD64 mechanism crate for low-level CPU facts and validation helpers.
* Mirage has a Ryzen descriptor crate that models CPUID-derived family, model, stepping, topology, and telemetry channel identifiers as mechanism data.
* Mirage has supervisor-facing platform service launch descriptions for AMD chipset, AMD IOMMU, and AMD telemetry services.
* Mirage has capability-mediated handoff records for AMD chipset and AMD IOMMU services.
* Mirage keeps AMD platform policy above the kernel: the platform layer prepares launch and handoff records, while the kernel remains responsible only for enforcing primitive capabilities.

## Mocked now

* CPU discovery is represented by typed descriptors rather than live CPUID probing across all Ryzen generations.
* Topology, chipset identity, IOMMU unit identity, MMIO ranges, IRQ lines, and DMA table ranges are supplied as structured handoff data.
* Service launch planning is declarative; no full boot-time AMD platform manager currently enumerates the complete host and starts every service automatically.
* Telemetry channels are named, but live sensor reads are not implemented.
* PPR-specific behavior is documented as a requirement, not encoded as a complete per-family/per-model rule table.

## Real hardware path

* Early architecture probing reads CPUID vendor, family, model, stepping, and feature bits.
* PCI discovery walks buses and records vendor IDs, device IDs, class codes, BARs, MSI/MSI-X capability state, and interrupt routing.
* The supervisor validates discovered AMD devices against public AMD references and the relevant processor PPR before granting service capabilities.
* The supervisor launches AMD platform services with least-privilege capabilities for PCI devices, MMIO regions, IRQ lines, DMA buffers, and IPC endpoints.
* Restartable driver services own device-specific work such as chipset coordination, IOMMU programming, and telemetry collection.
* Kernel participation stays limited to CPU privilege boundaries, page tables, interrupts, IPC transport, DMA/MMIO capability enforcement, and module-loading mechanisms.

## Unsupported areas

* Unsupported processors include any AMD CPU family/model/stepping combination without enough public documentation to build a safe service policy.
* Unsupported chipsets include devices with unknown PCI IDs, undocumented BAR layouts, or no public register description sufficient for controlled operation.
* Unsupported configurations include firmware tables or PCI topologies that cannot be validated against Mirage capability and service-isolation rules.
* Mirage does not promise Linux driver compatibility, ACPI driver reuse, or opaque firmware workarounds as part of the base AMD platform support.
* Overclocking, voltage tuning, vendor-private management features, and proprietary telemetry paths are outside the initial support target.

## Next steps

* Add a boot-time AMD platform discovery report that records CPUID, feature bits, PCI IDs, class codes, BARs, and firmware table references.
* Add supervisor policy tables keyed by CPU family, model, stepping, PCI vendor/device ID, and feature bit sets.
* Add explicit service manifests for `chipsetd`, `iommud`, and `amd-telemetryd`.
* Add tests showing that unsupported or under-documented AMD devices receive no MMIO, IRQ, DMA, or service-control capabilities.
* Add hardware bring-up notes per tested Ryzen platform with links to the public references used for that exact platform.
