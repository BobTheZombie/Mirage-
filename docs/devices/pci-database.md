# PCI Device Database

The PCI database records vendor, device, subsystem, class, subclass, and programming-interface matches used by PCI enumeration and driver selection.

## Required fields

PCI descriptors should include `vendor_id`, optional `device_id`, optional `subsystem_vendor_id`, optional `subsystem_device_id`, `class`, `subclass`, and optional `prog_if`. Class-only matches are allowed for generic drivers but must be lower priority than exact vendor/device matches.

## Driver hints and quirks

PCI hints may reference kernel-adjacent drivers such as `ahci`, `nvme`, `xhci`, or service-backed stacks such as `storaged` and `usbd`. Quirks must describe observable hardware behavior, not policy. Drivers must validate BARs, MMIO ranges, IRQ routing, DMA requirements, and command/event flow before reporting ONLINE.

## Fallback behavior

Unknown PCI devices should remain unbound or use a generic class driver only when that driver can safely probe with bounded waits. Optional device failure must degrade cleanly and must not block PID1 handoff.

## Provenance

Imported PCI IDs must identify their source and license. Public specifications, vendor documentation, QEMU documentation, and locally observed enumeration may be used. Do not copy incompatible database content without review.
