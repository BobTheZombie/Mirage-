# AMD Ryzen 5 4500U / Renoir USB Platform Audit

## Scope

This audit covers the current Mirage platform discovery, AMD/Ryzen scaffolding,
PCI/ACPI integration, xHCI/USB/HID bring-up, and keyboard input path.

## Working pieces

- Boot already reaches the x86_64 architecture initialization path, configures
  serial, CPU mode, memory layout, framebuffer, IDT/PIC/interrupt basics, then
  runs platform probes before storage and input initialization.
- CPUID probing decodes vendor, effective family/model, stepping, and recognizes
  `AuthenticAMD`; family `0x17`, model `0x60` is named `AMD Ryzen 5 4500U`.
- PCI enumeration reads sane vendor/device/class/subclass/prog-if/header fields
  through bounded legacy CF8/CFC bus-0 scanning and respects multifunction rules.
- The Platform Registry records discovered CPU, PCI, ACPI, and i8042 facts before
  driver/service lifecycle state is reported.
- Registry class helpers identify xHCI (`0x0c/0x03/0x30`), NVMe, AHCI, AMD SoC
  devices, AMD xHCI, and Renoir GPU candidates.
- Boot phases for absent optional hardware are intended to be skipped rather than
  failed, preserving QEMU and VirtualBox boot.
- PS/2 keyboard input is independent from the USB path and continues to publish
  common Mirage keyboard events.
- USB keyboard report diffing, HID usage-to-key translation, and the shared input
  event queue already exist.

## Stubbed or partial pieces

- ACPI support currently records RSDP presence but does not fully walk XSDT/RSDT,
  FADT, MADT, MCFG, or IVRS in the early architecture path.
- AMD IOMMU is detected only as a scaffold from ACPI/AMD SoC presence; global DMA
  translation is intentionally not enabled.
- AMDGPU Renoir is a PCI discovery/stub path only. Mirage must not reset or bind
  the GPU until a safe display driver service exists.
- xHCI initialization can map BAR0 through HHDM, enable PCI memory/bus mastering,
  read capability/operational offsets, halt/reset/run the controller with bounded
  waits, and install static DCBAA/command/event backing, but full command/event
  completion processing remains incomplete.
- USB Core can scan and reset root ports with timeouts, but it does not yet run the
  complete xHCI Enable Slot, Address Device, control transfer, descriptor read,
  Set Configuration, and endpoint configuration sequence.
- USB HID and USB Keyboard are honest now: they do not fabricate HID keyboards
  from merely connected ports. They remain skipped until real descriptor-backed
  HID binding is implemented.

## Broken pieces and blockers to USB HID keyboard

1. Full xHCI command/event ring ownership is missing: command completion matching,
   transfer event handling, and port status event handling need interrupt or polling
   integration.
2. Device context allocation and input/output context setup are not complete.
3. Control transfers are not yet submitted through transfer rings, so device and
   configuration descriptors cannot be read from hardware.
4. HID `SET_PROTOCOL`, `SET_IDLE`, and interrupt-IN endpoint scheduling require the
   missing control/endpoint configuration work.
5. ACPI MCFG is not parsed into an ECAM backend yet, so PCI probing remains bus-0
   CF8/CFC only.
6. IVRS parsing is not connected to DMA domain policy; this is intentionally a stub
   to avoid unsafe global IOMMU enablement.

## Duplicated logic found

- PCI enumeration existed in the platform path and an xHCI driver-local scanner.
  The USB stack now accepts the platform registry snapshot and selects xHCI from
  the registry, avoiding duplicate PCI probe logs in the normal boot path.
- Boot phase updates were performed both by the USB module runner and by the outer
  architecture input initializer. The module runner still owns detailed lifecycle
  decisions, while the outer layer mirrors final statuses for existing boot-screen
  compatibility.

## Unsafe MMIO/DMA areas

- xHCI MMIO register accesses are necessarily unsafe and must remain volatile.
- Static xHCI DMA structures are aligned, but a real DMA allocator and physical
  address translation contract are still required before robust transfers.
- BAR values are discovery facts, not authority grants; future driver services must
  receive explicit capabilities for MMIO, DMA, IRQ/MSI, and PCI configuration.

## Infinite wait risks

- The current hardware waits for controller halt, reset, run, port reset, and port
  enable are bounded by `WAIT_LIMIT` and return timeout errors instead of spinning
  forever.
- Future command completion and transfer waits must follow the same bounded pattern.

## Current support status

- QEMU/VirtualBox without xHCI: xHCI and downstream USB phases are skipped without
  starting absent hardware.
- QEMU/VirtualBox with xHCI: xHCI may reach Online if controller init succeeds;
  USB Core may come Online after bounded port scanning; HID/USB keyboard remain
  skipped unless real descriptor-backed enumeration is completed.
- Ryzen 5 4500U/Renoir: CPU and PCI inventory can identify AMD/Renoir candidates;
  AMD xHCI can be selected from Platform Registry; AMDGPU remains stubbed; IOMMU
  remains detected/stubbed or skipped.
