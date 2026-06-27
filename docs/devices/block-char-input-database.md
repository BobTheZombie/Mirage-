# Block, Character, and Input Device Database

This database describes higher-level device identities that are not fully captured by PCI or USB IDs, including discovered block devices, console/serial devices, and input devices.

## Block devices

Block descriptors may include transport, sector size, capacity constraints, removable state, partition expectations, and safe driver hints such as `ahci`, `nvme`, `atapi`, or `storaged`. Drivers must validate media state and bounded I/O before service status advances.

## Character devices

Character descriptors may include console, serial, terminal, debug, or service-channel identities. Capabilities should identify who may read, write, or control each endpoint.

## Input devices

Input descriptors may include scan-code set, HID usage page, keyboard layout hint, controller path, queue size, and overflow policy. Unknown scancodes must not panic, input queues must be bounded, and input failure must degrade rather than block boot.

## Fallback behavior

A descriptor may request `generic-readonly`, `diagnostic-only`, `disable-device`, or `unsupported` fallback. These are mechanism outcomes; Supervisor policy decides whether services continue, restart, or expose the device to userspace.

## Provenance

Device behavior can come from public specs, observed hardware, or Mirage-owned tests. Every imported mapping or quirk needs source and license metadata.
