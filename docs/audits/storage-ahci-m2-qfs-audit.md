# Mirage Storage / AHCI / M.2 / QFS Audit

## Scope

This audit covers the current Mirage storage path from platform discovery through PCI storage controllers, AHCI/NVMe driver crates, the block abstraction, QFS, root filesystem policy, and QEMU storage targets.

## What currently works

- `mirage-platform` has a fixed-capacity `PlatformRegistry` and records PCI class/subclass/prog-if plus BAR metadata captured by the architecture PCI pass.
- PCI class helpers identify AHCI (`01:06:01`), NVMe (`01:08:02`), and xHCI without requiring driver-local PCI rescans.
- `mirage-block` provides backend-independent block ranges, request queues, scheduler dispatch, validation of LBA ranges, and read/write buffer-size checks.
- AHCI and NVMe crates contain bounded command/queue models, identify-data parsers, and `BlockDevice` implementations for hardware feature builds.
- QFS has a block-device-backed host path, superblock parsing, root lookup tests, and hosted `qfsprogs` image formatting/validation.
- The boot phase manager can distinguish skipped, detected, started, failed, and online phases.

## What was stubbed or incomplete

- The x86_64 boot storage dispatch previously reported AHCI/NVMe as `Stub` when controllers existed instead of performing honest `Detected -> Started -> Online/Failed` transitions.
- Hardware driver crates still model MMIO/DMA queues in Rust data structures; the early architecture path does not yet map the controller and register real namespaces/disks during boot.
- QFS can mount over generic block adapters in hosted tooling, but kernel root selection is not fully wired to a global early-boot block registry.
- There was no documented `tools/qfs-mkimage/` entry point, though `qfsprogs` already implemented core image operations.

## What is unsafe or risky

- Driver-local PCI rescans would risk duplicate hardware ownership. Storage drivers must consume only `PlatformRegistry` records.
- Reporting `Online` before at least one block device is registered is unsafe. AHCI Online requires a registered SATA disk; NVMe Online requires a registered namespace.
- Writes must remain explicit. Root probing and QFS mounting must read superblocks and metadata only.
- Infinite controller waits are not acceptable. AHCI/NVMe queue polling must stay bounded and return `Timeout`/driver failure.

## Duplicated concepts

- QFS host tooling and the supervisor storage service both adapt block I/O to sector I/O; they should converge on `mirage-block` as the stable interface.
- Existing docs used both legacy flat storage docs and newer per-subsystem storage docs. New docs under `docs/storage/` are canonical.

## Fixes required before true QFS-on-real-block boot

1. Map AHCI BAR5/NVMe BAR0 from `PlatformRegistry` BAR records in the architecture storage path.
2. Enable PCI memory space and bus mastering through platform-owned PCI command helpers.
3. Allocate DMA-safe command/FIS/queue memory from the kernel DMA allocator instead of regular Rust-owned buffers.
4. Register discovered `sataN` and `nvme0nN` devices in the fixed-capacity block registry only after a successful identify/read sanity path.
5. Connect `root=auto|nvme0n1|sata0|builtin-qfs` to the block registry and QFS block mount API.
6. Keep fallback `BuiltInBlockQfs` explicit and never report physical M.2 form factor without ACPI/SMBIOS evidence.
