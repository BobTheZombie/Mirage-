# Mirage xHCI USB Storage Status

## Current real support level

Mirage does not currently provide production-ready xHCI or USB mass-storage real
hardware support. The default USB storage path models devices, endpoints,
transfers, SCSI READ(10)/WRITE(10), and block I/O with in-memory storage. The
`hw-xhci` feature adds xHCI-shaped PCI, register, ring, interrupter, port, slot,
and USB-storage controller state, but it does not enumerate real USB devices or
perform real xHCI transfers.

Do not claim real USB storage support until Mirage enumerates a USB mass-storage
device and performs verified block I/O against real hardware or a validated xHCI
emulator.

## What code is actually implemented

* USB endpoint, transfer, completion, device-class, and mass-storage transport
  metadata.
* A mock USB storage device that implements SCSI-like read/write commands over an
  in-memory block store.
* A mock xHCI controller path that can attach and expose mock USB storage
  devices.
* `hw-xhci` types for capability-checked xHCI resources, operational registers,
  runtime interrupter state, TRBs, transfer rings, ports, slots, and controller
  state.
* PCI class validation and BAR-shaped MMIO metadata handling for xHCI devices.

## What remains mocked

* Real xHCI MMIO register programming and controller reset/run/stop sequencing.
* DCBAA, command ring, event ring segment table, transfer rings, and scratchpad
  buffers backed by real DMA-safe memory.
* Port reset, slot enable/address, endpoint configuration, descriptor parsing,
  route strings, hubs, and hotplug.
* USB Bulk-Only Transport CBW/CSW execution and UAS are not implemented against
  real devices.
* SCSI inquiry, capacity, sense data, removable-media behavior, cache flush, and
  error handling are placeholders.
* Interrupts and event-ring processing are modeled rather than handled from real
  xHCI hardware.

## Intended hardware or emulator target

The first target should be a validated QEMU xHCI controller with a USB
mass-storage device backed by a scratch image. Validation should start with
controller reset, port enumeration, descriptor reads, and read-only capacity and
known-pattern block checks. Real USB sticks or SSD enclosures must wait until
malformed-device handling, DMA isolation, hotplug recovery, and scratch-write
controls exist.

## Known safety limitations

* USB devices can be malicious or malformed; the current skeleton is not hardened
  against hostile descriptors or transfer behavior.
* Real xHCI DMA structures are not allocated or isolated yet.
* Writes are safe only in the mock in-memory storage device.
* Hot-unplug, stalled endpoints, reset storms, and transfer timeouts are not
  safely recovered.
* There is no validated path to stop DMA and revoke all endpoint/controller
  authority after a service crash.

## TODO roadmap

1. Add a documented QEMU xHCI plus USB-storage validation recipe.
2. Implement checked xHCI MMIO reset/run/stop operations.
3. Allocate DCBAA, command ring, event ring, transfer rings, and scratchpad
   structures from DMA-safe memory.
4. Implement event-ring polling before enabling interrupts.
5. Implement port reset, slot enable/address, descriptor fetch, and endpoint
   configuration for one mass-storage device.
6. Implement USB Bulk-Only Transport READ CAPACITY and READ(10) in polling mode.
7. Add guarded WRITE(10) and SYNCHRONIZE CACHE only for scratch images.
8. Add IRQ-driven events, hotplug handling, reset recovery, and malformed-device
   limits.
9. Add supervisor crash cleanup for DMA, ports, slots, endpoints, and service
   registration.
