# Mirage PCI/MMIO/DMA Driver Foundation

Mirage drivers must share PCI, MMIO, DMA, interrupt, and timeout primitives instead of duplicating low-level logic in each controller driver.

The common `mirage-pci` model provides PCI class matching, BAR decoding, MMIO BAR validation, a bounded probe result model, tick-bounded polling helpers, and alignment-checked DMA buffer descriptors. Hardware-specific mapping still belongs in the lower kernel; driver crates consume validated descriptors and must not bypass capability checks.

Required status semantics:

- `Bound` means the driver validated and bound real hardware.
- `NotSupported` means absent hardware or a non-matching class/vendor/device tuple.
- `Degraded(reason)` means a nonfatal partial state, such as polling mode while IRQ work remains pending.
- `Failed(error)` means matched hardware failed a bounded real init step.

Polling rules:

- Never use unbounded MMIO polling loops.
- Every wait has a documented operation name and maximum tick count or timeout.
- Optional PCI devices must fail or degrade without blocking AHCI, PS/2, rootfs, Supervisor, MTSS, or PID1.
