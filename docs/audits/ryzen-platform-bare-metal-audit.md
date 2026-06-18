# Ryzen Platform Bare-Metal Audit

## Fault causes found

- CPU probing in the architecture path assumed CPUID leaf 1 and topology availability instead of recording max leaves first.
- Ryzen topology status was gated on Intel-style standard leaf `0x0b`, which is not the correct early requirement for Renoir AMD topology.
- AMD SoC, AMDGPU Renoir, and AMD xHCI detection statuses overclaimed `Ok` for metadata-only PCI presence.
- PCI probing printed noisy raw bus data by default, which made fault-localization logs harder to read.
- Optional MSR-backed Ryzen telemetry had no #GP-safe early probe path and is therefore skipped.

## Current safe boundary

CPUID-only CPU and topology facts are safe during early platform discovery. ACPI/MADT parsing requires mapped firmware pages. PCI BARs require explicit MMIO mapping before use. Device initialization belongs to later driver paths and supervised services.

## Remaining blockers

- Recoverable #GP-safe MSR probe support.
- Mapped and validated ACPI table walker for IVRS/MADT after memory mapping is complete.
- Bounded PCI bridge enumeration beyond bus 0.
- Real driver-service handoff for xHCI, AMDGPU, IOMMU, AHCI, and NVMe without marking metadata-only detections as online.
