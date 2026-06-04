# Mirage Memory Ownership

This document defines the current and intended ownership boundaries for physical
memory, virtual memory, memory objects, MMIO, and DMA in GNU/Mirage. The rule is
that the kernel owns machine accounting and enforcement, while the supervisor
owns policy about which service receives which authority.

## Implemented now

* Limine is the boot-time source of memory topology. The bootloader provides the
  memory map that the kernel consumes during early boot; Mirage does not invent
  firmware memory ranges or delegate first accounting to userspace.
* The kernel boundary is the owner of physical memory accounting. Physical pages,
  reserved boot ranges, executable image ranges, module ranges, framebuffer
  ranges, and other bootloader-described regions are kernel-managed resources.
* Capability checks already model the Mirage rule that services cannot receive
  raw unrestricted memory access. Memory-like authority is represented as scoped
  rights rather than ambient pointer access.
* The existing hardware scaffolds treat MMIO and DMA as explicit driver inputs,
  not as implicit global privileges. Storage, USB, framebuffer, and GPU-facing
  code paths document that real hardware access must be mediated by capabilities.

## Stubbed now

* Supervisor memory-object allocation is still architectural scaffolding. The
  intended call shape is that the supervisor requests kernel-created memory
  objects and then grants service capabilities for those objects.
* Service memory ownership is currently modeled rather than production-enforced
  end-to-end. A service should receive a memory capability that names the object,
  range, access mode, and lifetime; early demos may still use in-memory mocks.
* Driver MMIO and DMA authority is partially represented by capability metadata
  and feature-gated hardware skeletons. Drivers should receive MMIO capabilities
  for register mappings and DMA capabilities for bounded transfer memory, but the
  full IOMMU/cache-coherency/revocation implementation is not complete.
* Crash cleanup for memory capabilities is planned in the supervisor model, but
  complete page reclamation, DMA quiescing, and map teardown are not production
  mechanisms yet.

## Planned next

* Convert the Limine memory map into a stricter physical-frame database with
  explicit states for free, reserved, kernel, boot module, framebuffer, MMIO,
  DMA, service-owned, and revoked frames.
* Add a kernel memory-object primitive that can be requested by the supervisor,
  mapped into service address spaces, transferred over IPC only when permitted,
  and revoked on service crash or policy change.
* Define service grants for normal memory objects separately from driver grants
  for MMIO and DMA so ordinary services cannot accidentally acquire device-like
  authority.
* Add DMA-safe allocation and teardown rules: bounded regions, device ownership,
  IOMMU domain hooks where available, cache-coherency notes, and mandatory revoke
  behavior before driver restart.
* Document the future path as:

```text
Limine memory map
    -> kernel physical accounting
    -> supervisor memory-object request
    -> service memory capability
    -> driver MMIO/DMA capability when hardware access is required
```
