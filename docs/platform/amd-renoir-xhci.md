# AMD Renoir xHCI lower-kernel support

Mirage treats the Ryzen 5 4500U / Renoir USB3 host controller as the first real hardware-backed Renoir SoC driver path. The lower kernel owns PCI discovery, BAR/MMIO access, volatile register access, DMA/ring memory, controller halt/reset/start, root-port status, and polled operation. The supervisor owns policy: whether the controller may initialize, whether it becomes visible to userspace services, HID routing, and one bounded reset retry.

## Implemented hardware path

* PCI xHCI matching uses the serial-bus / USB / xHCI class triple, with Renoir inventory preferring AMD chipset/display vendor IDs without pinning support to one device ID.
* BAR0 is read from PCI config space and the boot path rejects missing, I/O, or zero BARs with a precise MMIO discovery failure.
* xHCI capability registers are parsed for capability length, HCSPARAMS1, HCCPARAMS1, DBOFF, RTSOFF, MaxSlots, MaxPorts, and context size.
* The controller path halts, waits for HCH, resets, waits for reset clear, configures DCBAA / command ring / event-ring backing, programs MaxSlotsEn, starts the controller, and waits for HCH clear.
* Root hub ports are scanned in polled mode. Connected ports are reset with bounded timeout loops and the xHCI port speed field is captured.

## Honest status model

`AMD XHCI [DETECTED]` means platform facts identify a Renoir-class AMD xHCI candidate. `XHCI [STARTED]` means lower-kernel MMIO and controller start succeeded. `XHCI [ONLINE]` is reserved for a completed command/event-path smoke test; the current boot keyboard path does not claim it merely because reset/start worked. HID keyboard remains skipped or pending unless a real device and endpoint path is configured.

## Out of scope

AMDGPU and AMD IOMMU translation are detection-only in this milestone. The GPU is not reset or modeset. IOMMU translation is not enabled until Mirage has a complete DMA remapping path.
