# Ryzen Platform Fault Hardening

The Ryzen platform path is split into explicit phases to prevent early bare-metal faults.

1. `IDT`: install exception gates only.
2. `PIC`: legacy PIC setup only.
3. `AMD64 CPU`: CPUID-only CPU facts.
4. `Ryzen CPU`: safe CPUID classification only.
5. `Ryzen Topology`: CPUID topology only, with single-core and no-SMT fallbacks.
6. `ACPI Tables`: only after firmware pages are mapped and validated.
7. `MADT/APIC`: only after ACPI tables are safe to read.
8. `AMD SoC`: PCI config inventory only after PCI config access is established.
9. Driver paths: xHCI, IOMMU, AMDGPU, AHCI, and NVMe remain separate from CPU detection.

## MSR policy

Optional Ryzen telemetry is skipped by default with the reason `unsafe until #GP-safe MSR probe exists`. Mirage does not issue optional MSR reads during basic CPU/Ryzen/topology discovery because a bad MSR can raise #GP and the early kernel does not yet provide recoverable MSR probing.

## PCI policy

The early PCI inventory uses legacy CF8/CFC config reads only. It records vendor/device/class/function metadata and BAR values as data, but it does not dereference BARs as MMIO during CPU discovery. Raw PCI dumps are disabled unless `MIRAGE_DEBUG_PCI` is set at build time.

## Breadcrumbs

The boot path emits concise serial/framebuffer breadcrumbs `[ryzen 01]` through `[ryzen 11]` around CPU facts, topology, MSR-skip, platform commit, and PCI inventory completion.
