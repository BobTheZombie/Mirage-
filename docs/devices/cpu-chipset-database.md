# CPU and Chipset Database

The CPU/chipset database records processor family/model/stepping, CPUID feature bits, chipset IDs, platform controllers, and known boot-critical quirks.

## Required fields

Descriptors should include architecture, vendor string, family, model, stepping range when relevant, feature flags, chipset or southbridge identifiers, and platform notes. Entries may also reference firmware interface requirements and interrupt-controller expectations.

## Driver hints and quirks

Hints may select timer, interrupt, IOMMU, PCI root, or platform services. Quirks must be tied to documented behavior or observed hardware evidence. Kernel code may use generated tables to choose bounded mechanism paths, but policy decisions remain above the kernel.

## Fallback behavior

Unknown CPU or chipset combinations should use conservative defaults and explicit degraded status. Mirage must not claim platform features are ONLINE merely because a family/model matched.

## Provenance

Use public CPU manuals, vendor programming guides, firmware tables, QEMU platform documentation, and observed boot logs. Document exact sources and avoid copying incompatible implementation code.
