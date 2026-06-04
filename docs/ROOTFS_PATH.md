# Mirage Root Filesystem Path

This document describes the intended path from boot media to the Mirage root
filesystem. QFS is the native root filesystem concept, and storage discovery must
flow through supervised services and capabilities rather than direct Unix-style
kernel device nodes.

## Implemented now

* QFS is the native Mirage indexed object filesystem and default architectural
  root filesystem target. It is modeled as Library / Book / Chapter / Page /
  Sector, not merely as a renamed Unix directory tree.
* Storage-facing drivers are expected to expose generic block devices or storage
  handles. QFS must not call USB, NVMe, AHCI, or other driver internals.
* Existing scaffolding includes mock block devices, storage service registration
  paths, QFS object lookup concepts, and Limine module-backed read-only block
  plumbing for boot-media experiments.

## Stubbed now

* The complete root path is not production-ready. Mirage can model pieces of the
  chain, but real block-device discovery, storage service launch, QFS mount,
  boot module lookup, and supervisor service launch are not yet one hardened
  boot path.
* Storage services still rely on deterministic mocks or feature-gated hardware
  skeletons for most device classes. They should receive device, MMIO, DMA, and
  IRQ capabilities before accessing real hardware.
* QFS mount policy is still supervisor-directed architecture work. The kernel
  should provide mechanisms and enforcement, while the supervisor chooses the
  root candidate and grants filesystem/service capabilities.

## Planned next

* Define the root device selection policy used by the supervisor, including boot
  manifest hints, storage service identity, QFS volume identity, fallback rules,
  and failure reporting.
* Connect supervised storage drivers to a generic storage registry that can
  expose block handles to QFS only after capability checks and service
  registration succeed.
* Implement QFS root mounting over a storage capability and make boot module
  lookup operate on QFS object metadata rather than hardcoded module arrays.
* Launch supervisor-managed services from QFS-resolved boot modules, preserving
  signed module validation and policy approval boundaries.
* Target the future root path:

```text
block device
    -> storage service
    -> QFS mount
    -> boot module lookup
    -> supervisor service launch
```
