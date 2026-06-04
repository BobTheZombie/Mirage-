# Mirage Real Hardware Support Status

This document is the top-level status page for Mirage hardware work. Mirage is
an experimental architecture skeleton. Hardware-facing crates are intentionally
split into deterministic mocks plus narrowly scoped, feature-gated hardware
skeletons. Do not describe Mirage as production-ready on real hardware unless a
specific driver initializes and performs I/O against that real device class, or
against a validated emulator that models that device class.

## Current real support level

| Area | Current level | Production-ready? |
| --- | --- | --- |
| NVMe | Mock block I/O is implemented. A feature-gated `hw-nvme` skeleton models PCI BAR discovery, capability checks, queue state, identify placeholders, and in-memory namespace data. It does not perform real NVMe DMA or controller I/O. | No |
| AHCI/SATA | Mock block I/O is implemented. A feature-gated `hw-ahci` skeleton models AHCI HBA/port state, FIS/PRDT descriptors, capability checks, and in-memory port data. It does not issue real SATA commands to hardware. | No |
| xHCI USB storage | Mock USB mass-storage and SCSI READ(10)/WRITE(10) storage are implemented. A feature-gated `hw-xhci` skeleton models PCI BAR discovery, controller register state, transfer rings, ports, slots, and mock USB storage. It does not enumerate or transfer with real USB devices. | No |
| Framebuffer | Safe in-memory framebuffer drawing is implemented. With `hw-framebuffer`, Mirage can wrap caller-provided framebuffer memory and write pixels through checked offset calculations. It does not discover firmware framebuffers by itself or provide production modesetting. | No |
| AMDGPU early display | Mock AMDGPU service behavior is implemented. With `hw-amdgpu`, Mirage can validate AMD PCI identity and supervisor-provided BAR metadata, create checked MMIO/VRAM descriptors, and preserve a boot framebuffer model. It does not initialize display engines, firmware, command processors, interrupts, or acceleration. | No |
| Serial console | A very small x86_64 COM1 UART driver uses port I/O for early serial output/input. It is not a broad hardware validation target for storage or graphics. | Not a production platform claim |
| Limine module block | A read-only block device can expose a Limine-loaded module as sectors. This is boot-media plumbing, not storage-controller support. | No |

## What code is actually implemented

* Capability-checked mock storage/device abstractions for NVMe, AHCI, USB
  storage, AMDGPU, and framebuffer scaffolding.
* Block device traits and storage-service registration paths that can be driven
  by deterministic in-memory devices.
* Feature-gated hardware skeleton modules for selected device families:
  `hw-nvme`, `hw-ahci`, `hw-xhci`, `hw-framebuffer`, and `hw-amdgpu`.
* Low-level x86_64 port I/O helpers used by legacy devices, with test/std-like
  builds returning inert values instead of issuing privileged instructions.
* A COM1 UART driver and a Limine module-backed read-only block driver.

## What remains mocked

* PCI enumeration as a real boot-time hardware bus scan for these drivers.
* Real MMIO register access for storage and USB controller operations.
* DMA-safe memory allocation, IOMMU domain setup, PRP/SGL/PRDT/TRB backing
  memory, and cache coherency handling.
* Real IRQ/MSI/MSI-X routing and interrupt-driven completion.
* Controller reset, power management, error recovery, hotplug, and teardown.
* Real NVMe admin/I/O commands, SATA FIS execution, USB enumeration, and USB
  bulk/UAS transfers.
* Real framebuffer discovery, EDID parsing, modesetting, GPU firmware loading,
  command processor initialization, display engine programming, and acceleration.

## Intended hardware or emulator targets

Initial validation targets should be explicit and narrow:

1. QEMU or another validated emulator exposing standard PCI NVMe, AHCI, xHCI,
   and boot framebuffer devices.
2. Single-device, read-only or scratch-device test configurations before any
   destructive write tests are enabled.
3. Real hardware only after emulator initialization, capability checks, DMA,
   interrupts, and error recovery are proven for the relevant driver.

A target is considered "validated" only when the driver initializes the device,
performs the advertised I/O path, checks data integrity, and documents the exact
emulator/device model and command line or hardware identity used.

## Known safety limitations

* Hardware feature flags are scaffolds, not safety certification.
* Storage write paths must be treated as destructive until bounded test media and
  recovery plans exist.
* DMA and MMIO mistakes can corrupt memory or hang a machine once real access is
  introduced.
* GPU register programming can blank displays, wedge devices, or require a
  reboot if reset handling is incomplete.
* USB hotplug and malformed-device behavior are not hardened.
* Capability checks model Mirage authority, but they do not yet replace missing
  IOMMU, interrupt, reset, and resource cleanup mechanisms.

## TODO roadmap

1. Document exact emulator targets for each hardware class.
2. Add supervised PCI enumeration and per-device capability issuance.
3. Add safe MMIO mapping ownership, lifetime, and revocation semantics.
4. Add DMA buffer allocation with IOMMU-aware bounds and cache-coherency rules.
5. Add polling-only read paths in validated emulators before enabling writes.
6. Add interrupt-driven completion after polling paths are correct.
7. Add crash cleanup: revoke capabilities, stop DMA, reset devices, reclaim
   queues/rings/buffers, and restart supervised services.
8. Add CI or hardware-lab recipes that record exact emulator/device validation
   evidence.
9. Update the per-driver status documents whenever a driver moves from mock, to
   emulator-validated, to real-hardware-validated.
