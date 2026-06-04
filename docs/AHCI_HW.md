# Mirage AHCI Hardware Status

## Current real support level

Mirage does not currently provide production-ready AHCI/SATA real hardware
support. The default AHCI implementation is a mock controller with in-memory
ports. The `hw-ahci` feature adds AHCI-shaped HBA, port, FIS, PRDT, command
header, and controller types, but command execution is still modeled and data is
stored in memory rather than read from or written to a real SATA device.

Do not claim real AHCI/SATA support until Mirage initializes an AHCI HBA and
performs verified block I/O against a real disk or a validated AHCI emulator.

## What code is actually implemented

* Capability-protected AHCI resource metadata for PCI, MMIO, DMA, and IRQ
  authority.
* Mock AHCI controller and block device paths that implement the common block
  interface using in-memory data.
* `hw-ahci` register-shaped structures for HBA memory and ports.
* SATA signature handling for ATA versus non-ATA placeholders.
* FIS Register Host-to-Device encoding and PRDT descriptor encoding helpers.
* A hardware-shaped controller wrapper that validates AHCI PCI identity and BAR
  metadata supplied by the caller.

## What remains mocked

* Real HBA MMIO programming, BIOS/OS handoff, HBA reset, port reset, and link
  bring-up are not implemented.
* Command list and received-FIS buffers are not allocated from real DMA-safe
  memory.
* Command issue/completion is simulated by software state.
* ATA IDENTIFY, READ DMA EXT, WRITE DMA EXT, cache flush, NCQ, and error recovery
  are not sent to real devices.
* Interrupts, port hotplug, staggered spin-up, ATAPI, port multipliers, SMART,
  trim/discard, and power management remain TODOs.
* Block contents are an in-memory vector, not SATA media.

## Intended hardware or emulator target

The first target should be QEMU's AHCI/SATA emulation using a scratch disk image.
Validation should start with a single ATA disk on one port, polling-only command
completion, and read-only known-pattern checks. Real SATA hardware must wait
until DMA allocation, HBA reset, error handling, and scratch-media write safety
are documented.

## Known safety limitations

* AHCI writes are safe only in the mock in-memory backend today.
* Real PRDT/DMA buffer mistakes can corrupt memory once hardware access exists.
* A failed port reset or bad command issue path can hang the HBA or disk.
* No tested recovery exists for task-file errors, link loss, or hotplug.
* The driver should never be pointed at valuable disks until emulator validation
  and explicit destructive-test controls exist.

## TODO roadmap

1. Add a documented QEMU AHCI validation recipe.
2. Implement checked MMIO access to the HBA and ports through supervisor-granted
   capabilities.
3. Implement HBA enable/reset and per-port stop/start sequencing.
4. Allocate command lists, command tables, PRDTs, and received-FIS areas from
   DMA-safe memory.
5. Implement ATA IDENTIFY in polling mode against a validated emulator.
6. Implement polling READ DMA EXT for one scratch disk.
7. Add guarded WRITE DMA EXT only after read validation and scratch-media checks.
8. Add interrupt-driven completions through IRQ capabilities.
9. Add crash cleanup: stop commands, revoke capabilities, reset ports, and
   re-register recovered devices through the supervisor.
