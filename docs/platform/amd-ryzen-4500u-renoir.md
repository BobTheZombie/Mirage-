# AMD Ryzen 5 4500U / Renoir Platform Notes

Mirage treats AMD Ryzen 5 4500U-class Renoir support as boot-safe discovery first, not as full hardware enablement.

## CPUID-only safe facts

Early `AMD64 CPU`, `Ryzen CPU`, and `Ryzen Topology` phases use CPUID only. The probe reads leaf `0` first, records the maximum standard leaf, reads extended leaf `0x80000000`, and then only reads leaves that are within those advertised ranges.

Safe CPUID facts include vendor string, family, model, stepping, optional brand string, APIC/x2APIC feature bits, XSAVE/OSXSAVE feature bits, invariant TSC, address widths, current APIC ID, and conservative topology counts.

Renoir-class detection is metadata-only: AMD family `0x17`, model `0x60..=0x7f` is classified as Renoir/Lucienne-class Zen 2 mobile. A Ryzen 5 4500U-class label is best-effort and requires a 6-core / 6-thread no-SMT topology.

## Hardware-backed states

`AMD64 CPU [Ok]`, `Ryzen CPU [Ok]`, and `Ryzen Topology [Ok]` mean CPUID parsing completed safely. PCI devices found later are `Detected`; that is not driver ownership and not online service health.

## Skipped or stubbed states

AMDGPU Renoir, AMD IOMMU, AMD PSP, AMD SMU, AMD xHCI, AHCI disks, NVMe namespaces, rootfs, and userspace must not be marked `Online` merely because a PCI function or metadata record exists.
