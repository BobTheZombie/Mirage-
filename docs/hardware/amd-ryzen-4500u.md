# AMD Ryzen 5 4500U / Renoir APU Hardware Support

Mirage now has feature-gated discovery modules for AMD Ryzen 5 4500U-class
Renoir laptops. The support is intentionally split across mechanism crates and
boot phases; it is not a single monolithic driver.

## Supported target

* CPU/APU: AMD Ryzen 5 4500U-class Renoir/Lucienne Zen 2 mobile APU
* Architecture: x86_64 / AMD64
* Integrated GPU: AMD/ATI Renoir Radeon Vega Mobile, PCI `1002:1636`
* Chipset/SoC inventory: AMD host/FCH, PSP, SMBus/I2C, xHCI, AHCI/NVMe,
  audio/ACP/DMIC, AMD IOMMU candidates
* Boot fallback: QEMU-safe; Ryzen-specific devices may be absent and must skip
  without failing boot

## Feature/config gates

Relevant Cargo features and MirageConfig symbols include:

* `hw-amd64` / `CONFIG_MIRAGE_HW_AMD64`
* `hw-ryzen` / `CONFIG_MIRAGE_HW_RYZEN`
* `hw-amd-chipset` / `CONFIG_MIRAGE_HW_AMD_CHIPSET`
* `hw-amd-iommu` / `CONFIG_MIRAGE_HW_AMD_IOMMU`
* `hw-amdgpu` / `CONFIG_MIRAGE_HW_AMDGPU_RENOIR`
* `hw-acpi` / `CONFIG_MIRAGE_HW_ACPI`
* `hw-acpi-ec` / `CONFIG_MIRAGE_HW_ACPI_EC`
* `hw-xhci`, `hw-nvme`, `hw-ahci`
* `hw-apu-renoir` / `CONFIG_MIRAGE_HW_RENOIR`
* `hw-ryzen-4500u` / `CONFIG_MIRAGE_HW_RYZEN_4500U`

`CONFIG_MIRAGE_HW_RYZEN_4500U` selects AMD64, Ryzen, Renoir, AMD SoC, ACPI, and
PCI support. AMD IOMMU depends on ACPI plus PCI. AMDGPU Renoir depends on PCI.

## Current initialized vs detected-only status

| Subsystem | Status |
| --- | --- |
| AMD64 CPUID | Hardware-backed CPUID when `hw-amd64` is enabled |
| Ryzen topology | Exposes topology and MTSS scheduler hints; no scheduler policy change |
| AMD SoC inventory | Classifies PCI candidates for supervisor policy |
| AMD IOMMU | Parses PCI IOMMU capability and ACPI IVRS header; translation is not globally enabled |
| ACPI | Bounds-checked parser for RSDP/RSDT/XSDT/FADT/MADT/HPET/MCFG/IVRS bytes |
| ACPI EC/Thermal/Battery | EC probing is conservative; AML-dependent thermal/battery methods skip until AML exists |
| AMDGPU Renoir | Detects PCI `1002:1636`, validates BAR metadata, preserves boot framebuffer; no reset/modeset |
| xHCI/NVMe/AHCI | PCI routing hooks to existing modular stacks; absence skips cleanly |

## Safety restrictions

* No infinite waits: hardware polling paths use bounded loops.
* No GPU reset or modeset in the Renoir path.
* No global IOMMU translation enable until DMA domains and device tables are safe.
* No ACPI AML assumptions: table parsing is supported, AML-dependent EC/thermal/battery features skip.
* No panic when laptop hardware is absent; QEMU should mostly report skipped Ryzen/APU phases.
* PCI BAR mappings must be validated against supervisor-provided resources before MMIO access.
* MMIO access remains volatile and capability-scoped through `mirage-hw` abstractions.

## Real laptop test checklist

1. Boot with serial logging enabled.
2. Confirm `[amd64] vendor=AuthenticAMD family=... model=...` appears.
3. Confirm `[ryzen] brand="AMD Ryzen 5 4500U ..."` and `cores=6 threads=6`.
4. Confirm AMD SoC inventory lists host bridge, PSP/SMBus/I2C if present, xHCI, storage, audio, and IOMMU candidates.
5. Confirm AMDGPU Renoir reports PCI `1002:1636` and does not reset or modeset the GPU.
6. Confirm ACPI table parser reports FADT/MADT/MCFG and IVRS when firmware provides them.
7. Confirm EC, Thermal, and Battery are `OK` only when actually supported; otherwise `SKIPPED`.
8. Confirm QEMU still boots and missing Ryzen laptop hardware does not fail boot.

## QEMU behavior

Generic AMD64 CPUID should work in an x86_64 QEMU environment. Renoir-specific
PCI devices, IVRS, EC, thermal zones, battery methods, and AMDGPU are normally
absent, so those phases should report `SKIPPED` or `DETECTED` rather than `OK`.
