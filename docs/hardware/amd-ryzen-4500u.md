# AMD Ryzen 5 4500U / Renoir Support Status

Expected discovery on real hardware:

- AMD64 CPU: Detected/Ok when CPUID vendor is `AuthenticAMD`.
- Ryzen CPU: Ok for family `0x17`/`0x19`; Ryzen 5 4500U is family `0x17`, model
  `0x60`.
- Ryzen Topology: Ok when CPUID topology leaves are available.
- AMD SoC: Ok when AMD vendor `0x1022` PCI functions are present.
- AMD xHCI: Ok/Online only when a real AMD xHCI controller initializes.
- AMDGPU Renoir: detected/stubbed; Mirage must not reset the GPU in early boot.
- AMD IOMMU: detected/stubbed or skipped; Mirage must not globally enable IOMMU
  translation until DMA domains exist.

Real laptop checklist:

1. Capture serial logs for CPUID and PCI inventory.
2. Confirm Renoir GPU (`1002:1636`/`1002:1638`) is only detected, not reset.
3. Confirm AMD xHCI BAR0 is present and HHDM-mapped.
4. Confirm all xHCI waits timeout rather than freezing if hardware misbehaves.
5. Test external USB keyboard separately from any internal PS/2/i8042 keyboard.
