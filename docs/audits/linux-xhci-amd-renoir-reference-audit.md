# Linux xHCI / AMD Renoir Reference Audit

This audit records Linux source files inspected as behavioral/provenance references for Mirage's independent Rust xHCI/USB/Renoir bring-up. No Linux GPL C code was copied into Mirage.

## Linux files inspected

From `https://github.com/torvalds/linux` (`master`, inspected 2026-06-23):

- `drivers/usb/host/xhci.c`
- `drivers/usb/host/xhci.h`
- `drivers/usb/host/xhci-pci.c`
- `drivers/usb/host/xhci-ring.c`
- `drivers/usb/host/xhci-mem.c`
- `drivers/usb/host/xhci-hub.c`
- `drivers/usb/host/xhci-ext-caps.c`
- `drivers/usb/core/hub.c`
- `drivers/usb/core/message.c`
- `drivers/usb/core/config.c`
- `drivers/usb/core/urb.c`
- `drivers/usb/storage/usb.c`
- `drivers/pci/pci.c`
- `drivers/acpi/pci_root.c`
- `drivers/iommu/amd/init.c`
- `arch/x86/pci/common.c`
- `arch/x86/kernel/setup.c`

## Behavior learned

- xHCI controllers are matched by PCI serial-bus/USB/xHCI class tuple `0x0c/0x03/0x30`; vendor/device IDs are diagnostics and quirk provenance, not the primary generic bind key.
- Bring-up order is PCI command enable, MMIO capability validation, halt, host-controller reset, context/ring allocation, DCBAA/CRCR/ERST/interrupter setup, slot configuration, start, then command/event validation.
- Event and command rings use producer/consumer cycle bits; command completion events are the first useful liveness proof before USB devices are marked usable.
- Root hub enumeration must inspect each PORTSC register, reset only connected ports, bound every wait, and preserve boot if a port or device fails.
- Legacy ownership handoff, MSI/MSI-X, AMD IOMMU translation, scratchpad buffers, and USB3/USB2 roothub companion behavior require explicit platform support before Mirage can claim full coverage.

## Linux quirks considered

- AMD/Renoir controllers may require normal xHCI extended capability and BIOS/OS ownership handling before reset on firmware-heavy systems.
- MSI/MSI-X should have a polling fallback during early bring-up.
- Scratchpad buffers are mandatory when advertised by HCS parameters; Mirage currently reports an explicit failure rather than silently starting without them.
- 64-bit DMA is expected on modern Ryzen mobile systems, but Mirage still needs a real DMA allocator/IOMMU ownership model.

## Mirage implementation

Mirage implements an original Rust early xHCI path in `src/arch/x86_64/xhci_keyboard.rs`:

- PCI xHCI class matching and platform-selected controller binding.
- MMIO BAR validation and PCI memory/bus-master enable.
- xHCI capability length, structural parameter, runtime offset, and doorbell offset validation.
- Bounded halt/reset/start waits.
- Static early-boot DCBAA, command ring, event ring, ERST, and interrupter-0 setup with physical-address translation.
- A No-Op command submission and bounded command-completion event poll before `xhci-host0` is marked `ONLINE`.
- Root-port scan and reset with bounded waits; USB/HID/keyboard phases remain skipped unless real devices and class paths are proven.

## Intentionally not copied

No Linux functions, structures, comments, tables, or quirk code were copied. Linux was used only to confirm specification-derived sequencing and to identify areas requiring explicit Mirage policy.

## Remaining unknowns

- Real Dell Inspiron 15 5505/Ryzen 5 4500U scratchpad count, IOMMU mode, MSI routing, and firmware handoff behavior need hardware logs.
- USB descriptor control transfers, Address Device, HID Set Protocol/Idle, and USB Mass Storage BOT command transport are not yet complete in the early kernel path.
- A supervised `usbd` service should eventually own most of this logic after the kernel provides DMA/MMIO/IRQ capabilities.
