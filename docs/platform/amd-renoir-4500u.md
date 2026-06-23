# AMD Renoir / Ryzen 5 4500U Platform Path

Target hardware: Dell Inspiron 15 5505 FHD with AMD Ryzen 5 4500U (Renoir / Zen 2 mobile).

## Platform facts

- CPU/APU family is treated as Ryzen Zen 2 mobile Renoir when CPUID/model data matches the existing Mirage Renoir descriptors.
- AMD xHCI is associated through PCI class `0x0c0330` and AMD vendor IDs are retained as diagnostics.
- AMD IOMMU remains detected/pending unless a real remapping path is enabled.
- AMDGPU Renoir remains stub/skipped unless real display driver code executes.

## xHCI status contract

Mirage may report:

- `AMD XHCI [DETECTED]` after a matching PCI controller is found.
- `XHCI [ONLINE]` only after MMIO validation, reset/start, ring setup, and a command-completion event.
- USB core/HID/keyboard/storage are skipped or pending unless their real enumeration/configuration path succeeds.

## Real-hardware instructions

Boot a clean Mirage image on the Dell Inspiron 15 5505 and capture serial logs. Confirm that boot continues, framebuffer milestone UI remains live, and xHCI reports either a successful No-Op command-completion event or an explicit bounded failure such as scratchpad/DMA/ownership timeout.
