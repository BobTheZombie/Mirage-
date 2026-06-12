# Mirage Boot Milestone 1.1

Boot Milestone 1.1 extends the bootable Mirage skeleton with a canonical Boot
Phase Manager and a persistent framebuffer boot screen driven from registered
subsystem records.

## Included capabilities

- seed-rs handoff into `BootInfo` and `kernel_main`.
- BootInfo construction from Limine/seed-rs handoff data.
- x86_64 architecture bring-up.
- serial initialization.
- GDT, memory map, physical allocator, kernel mapper/paging, heap, and memory
  status reporting.
- framebuffer initialization and persistent boot screen rendering.
- IDT, PIC, and interrupt enablement status reporting.
- PS/2 keyboard, xHCI/USB keyboard, ACPI EC, EC hotkey, and input subsystem
  visibility.
- supervisor creation, root filesystem mount attempt, supervisor bootstrap,
  userspace stub, MTSS initialization, boot screen, and idle-loop status.

## Boot Phase Manager scope

Milestone 1.1 includes:

- Boot Phase Manager as the canonical subsystem registration system.
- Static subsystem registration before initialization.
- explicit `Registered`, `Started`, `OK`, `Online`, `Enabled`, `Pending`,
  `Failed`, `Skipped`, and `Stub` state reporting.
- duplicate-registration and missing-registration diagnostics.
- weighted progress calculation from registered subsystem records.
- persistent framebuffer boot screen rendering from the registered subsystem
  table instead of a hardcoded boot-status list.
- serial transition logs for every state change.

## Architectural rule

The Boot Phase Manager remains a mechanism-level visibility table. It does not
own recovery, launch authorization, service policy, capability grants, or root
filesystem policy. Those decisions remain with the Mirage supervisor and the
appropriate kernel/MTSS layers.
