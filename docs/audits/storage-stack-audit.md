# Mirage Storage Stack Audit

Date: 2026-06-13

## Scope

This audit covers the requested boot path:

```text
PCIe Bus -> Platform Registry -> NVMe Controller -> NVMe Namespaces ->
Mirage Block Layer -> QFS / ext4 -> RootFS -> Supervisor -> MTSS ->
Spider-rs -> Userspace
```

The key architectural constraint is preserved: **M.2 is a physical form factor, PCI Express is the transport, and NVMe is the storage protocol**. Mirage must not grow a separate "M.2 driver". Any M.2-capable storage status may only mean that a PCIe NVMe/AHCI storage path is present unless firmware/platform data proves the physical connector.

## Executive Summary

Mirage currently has useful storage-facing abstractions and several deterministic mock paths, but it does **not** have complete production-grade hardware-backed storage bring-up. The largest risks are fake hardware success paths in feature-gated NVMe/AHCI models, incomplete BAR sizing/readback diagnostics in generic PCI plumbing, descriptor-only DMA validation, and root filesystem policy that can report architectural progress before a real hardware-backed block device has been proven online.

This audit resulted in two immediate hardening changes:

1. DMA descriptors in `mirage-hw` now reject unaligned physical addresses, virtual addresses, and lengths instead of accepting sub-page buffers as hardware DMA resources.
2. Kernel MMIO verification now walks page tables and validates present, writable when requested, and kernel-only mappings instead of only checking that a virtual address translates.
3. Generic PCI BAR parsing now has a sized BAR decoder for probe masks so callers can record width/type/base/length without inventing fallback lengths.

## Status Matrix

| Area | Current state | Production gap | Required next step |
| --- | --- | --- | --- |
| PCI discovery | x86_64 CF8/CFC probing exists and classifies NVMe as class `01/08/02`; generic `mirage-pci` can parse config-space snapshots. | Legacy config probing is not enough for all PCIe systems; ECAM/MCFG mapping, BAR size probing integration, command readback diagnostics, and capability-mediated claims are incomplete. | Add an architecture PCI config provider that supports MCFG/ECAM when ACPI supplies it and records BAR raw/base/size/type/width plus command before/after. |
| Platform Registry | Storage PCI devices can be surfaced as platform devices and named. | Logs do not guarantee the full required format and should not claim physical M.2. | Emit `NVMe PCIe SSD Controller` diagnostics with `PCI BB:DD.F`, vendor/device, and `01/08/02`; emit only `M.2-capable storage path: Detected/Online`. |
| MMIO mapper | Central `src/kernel/mmio.rs` maps page-aligned MMIO apertures. | Verification previously only checked translation; unmap and BAR-specific policy are not complete. | Add unmap/reference tracking and require all PCI drivers to consume `map_mmio`/`map_bar`, never HHDM+physical for BARs. |
| DMA allocator | `mirage-hw` models capability-granted DMA regions. | No production allocator exists that reserves contiguous frames, zeros memory, documents coherency, and returns typed queue/PRP buffers. | Implement a kernel/supervisor DMA allocator with page alignment, physical contiguity, zeroing, IOMMU domain hooks, and explicit cache coherency policy. |
| Block Layer | `mirage-block` validates ranges, sizes, queueing, and read/write/flush trait contracts. | It relies on device implementations for real I/O and timeout/error diagnostics. | Add standardized request timeout/error metadata and device health transitions; forbid `Online` until a backend proves real I/O. |
| AHCI | Mock and feature-gated structures exist. | Hardware path still uses simplified/fake completion behavior and does not perform full HBA/port bring-up. | Keep AHCI unavailable for production boot until PxCMD/PxTFD/PxCI/PxIS handling and DMA command tables are real. |
| NVMe | Mock and feature-gated structures exist, including command encoding, queue models, and class detection. | Feature-gated hardware model still fabricates identify data, namespaces, and completions; it does not perform volatile MMIO, doorbells, real DMA queues, or PRP I/O. | Replace fake completion injection with MMIO-backed CAP/VS/CC/CSTS/AQA/ASQ/ACQ/doorbell operations and timeout-bounded completion polling. |
| QFS | Block-backed QFS code exists with mount/read/write/journal-related structures. | It must be validated against real block devices and boot device selection. | Mount QFS only through registered block capabilities and add integration tests with a real image on AHCI/NVMe backends. |
| ext4 | no_std ext4-derived parser/writer primitives exist. | Full ext4 create/delete/rename/truncate and journal replay are not complete production semantics. | Treat ext4 as read/write-in-progress; mount read-write only after journal and metadata update ordering are proven. |
| RootFS | RootFS docs/configs exist. | `root=auto`, `root=qfs:nvme0n1`, `root=ext4:nvme0n1`, AHCI fallback, and builtin fallback need one policy engine with exact diagnostics. | Implement deterministic root selection: NVMe, AHCI, builtin QFS; report every failed probe with concrete error. |
| Supervisor | Supervisor owns service/capability policy and registers device resources. | It must not publish fake block/filesystem devices as online. | Publish storage devices only after hardware capability checks and successful identify/read/mount. |
| MTSS | MTSS crate exists for scheduler/service execution. | Storage boot does not prove Spider-rs is scheduled on MTSS after root mount. | Wire root mount completion into MTSS service-task launch and PID assignment. |
| Spider-rs loader integration | Spider-rs userspace crates exist. | PID 1 launch from mounted root is not fully proven hardware-backed. | Load Spider-rs from mounted QFS/ext4, create process, schedule via MTSS, and enter userspace with failure diagnostics. |

## Detailed Findings

### PCI discovery

Required NVMe detection tuple:

```text
class    = 0x01
subclass = 0x08
prog_if  = 0x02
```

Current generic PCI role detection matches that tuple. The x86_64 platform probe also names class `01/08/02` devices as NVMe controllers. This is correct for desktop PCIe NVMe cards, laptop M.2 PCIe SSDs, enterprise devices, QEMU NVMe, and future Ryzen laptops because it keys on PCI class/protocol rather than physical form factor.

Gaps:

- BAR sizing is not fully integrated into hardware enumeration.
- PCI command register enable/readback should be centralized.
- ECAM/MCFG is not the primary path yet.
- Discovery logs should explicitly state `NVMe PCIe SSD Controller`, not `M.2 NVMe`, unless firmware proves physical form factor.

### Platform Registry

Current registry entries can carry PCI location, identifiers, class fields, BAR descriptors, and IRQ line. Required production diagnostics are:

```text
[Platform] Device found:
    NVMe PCIe SSD Controller

    Location:
        PCI BB:DD.F

    Vendor:
        xxxx

    Device:
        xxxx

    Class:
        01/08/02

Boot phase:
    M.2-capable storage path:
        Detected
```

Finding: Mirage should report `M.2-capable storage path` as a capability of the storage path, not a physical assertion.

### MMIO mapper

Current `src/kernel/mmio.rs` provides `map_mmio()` and reserves a kernel virtual MMIO window. The audit hardening changed verification so every mapped page must have:

- present PTE
- writable PTE when write access was requested
- no user-accessible bit

Gaps:

- `unmap_mmio()` is not implemented.
- `map_bar()` is not yet a public typed helper.
- There is no mapping lifetime registry to prevent overlap/leak.
- Cache type policy is limited to cache-disable/write flags; PAT/MTRR policy remains future work.

### DMA allocator

Current DMA modeling is descriptor/capability-oriented. The audit hardening now rejects non-page-aligned DMA descriptors.

Gaps:

- No production allocator reserves physically contiguous frames for admin queues, I/O queues, PRP lists, AHCI command tables, xHCI rings, or AMDGPU buffers.
- No allocator-level zeroing proof exists.
- No explicit cache coherency / flush / invalidate contract exists.
- IOMMU integration is not wired into storage queue creation.

Production rule: a DMA descriptor is not enough. A real allocator must prove allocation, zeroing, contiguity, device authority, and IOMMU visibility.

### Block Layer

The block layer provides a good backend-neutral contract:

- `read_blocks()`
- `write_blocks()`
- `flush()`
- range validation
- buffer length validation
- read-only checks
- queue dispatch

Gaps:

- Hardware timeouts and device-specific status are collapsed too aggressively.
- Block devices can be created as `Online` by wrappers even when a lower fake backend fabricated success.
- Diagnostics should distinguish PCI/MMIO/DMA/queue/status errors.

### AHCI

AHCI has mock/service structures and capability checks. However, the production path remains incomplete for:

- HBA reset and global host control sequencing
- port discovery
- PxCMD/PxTFD/PxCI/PxIS polling with timeouts
- command list/FIS/PRDT physical addresses from real DMA memory
- actual data movement and error status interpretation

Finding: AHCI must remain non-production until fake completion paths are removed.

### NVMe

NVMe has mock/service structures and a feature-gated hardware module with command encoding, queue models, and validation helpers.

Critical production blockers:

- `identify_controller()` returns mock identify data.
- `identify_namespaces()` creates a synthetic `nvme0n1`.
- I/O completion is injected by pushing success completions into an in-memory queue.
- Registers are shadow fields, not volatile MMIO-backed CAP/VS/CC/CSTS/AQA/ASQ/ACQ/doorbell accesses.
- PRP lists use a single `dma_region` number rather than allocated physical pages and PRP chain validation.

Required NVMe bring-up sequence:

1. Validate PCI class `01/08/02`.
2. Size BAR0, verify it is memory BAR, enable Memory Space and Bus Master, and verify readback.
3. Map BAR0 through central MMIO.
4. Read CAP/VS; derive timeout, doorbell stride, queue entry sizes, and memory page size constraints.
5. Disable controller; wait for CSTS.RDY clear with timeout.
6. Allocate zeroed physically contiguous admin SQ/CQ DMA.
7. Program AQA/ASQ/ACQ.
8. Program CC with IOSQES/IOCQES/MPS/CSS and EN=1; wait for CSTS.RDY set with timeout.
9. Submit Identify Controller using PRP DMA buffer.
10. Submit Identify Active Namespace List.
11. Identify each namespace and parse LBA formats/capacity.
12. Create I/O completion/submission queues through admin commands.
13. Register each namespace as a block device only after a real command completes.

### QFS

QFS is the native Mirage filesystem and has block-backed code. It must not know about NVMe/AHCI internals; it should receive a block device capability and mount from that.

Production blockers:

- Real hardware-backed mount coverage is missing.
- Root selection and diagnostics need one policy path.
- Journal recovery semantics require validation against forced power-loss images.

### ext4

The ext4 module contains no_std structures and parser/writer primitives, but full production ext4 is a large filesystem implementation.

Production blockers:

- Journal replay awareness is not equivalent to safe journal replay.
- Create/delete/rename/truncate must update all metadata in correct order.
- 64-bit ext4, extents, block group descriptors, bitmaps, and inode table updates need image-level tests.

Policy: ext4 read-write root must remain disabled until journal and metadata ordering tests pass.

### Root filesystem mounting

Required boot arguments:

```text
root=auto
root=qfs:nvme0n1
root=ext4:nvme0n1
root=qfs:sata0
root=ext4:sata0
```

Required auto policy:

```text
NVMe -> AHCI -> builtin QFS
```

Production blockers:

- Device names must come from real namespace/disk registration.
- Mount should fail closed with exact diagnostics.
- Builtin fallback must be clearly labeled fallback, not hardware root.

### Supervisor

Supervisor must publish:

- mounted root
- filesystem registry
- storage devices
- block devices
- namespace registry
- service registry

Finding: publication must occur after real proof, not after constructor success. A driver service crash must revoke capabilities and remove/mark devices offline.

### MTSS

Required for final path:

- scheduler
- threads
- kernel tasks
- service tasks
- message queues
- IPC
- synchronization
- sleep/wake
- timer integration
- CPU-local execution

Finding: storage readiness should become an MTSS-visible supervisor event. Spider-rs must be a scheduled task/process, not a direct call.

### Spider-rs loader integration

Spider-rs should become PID 1 after root mount verification.

Required responsibilities:

- mount verification
- userspace loader
- service manager
- dependency ordering
- restart policy
- shutdown
- logging

Finding: Spider-rs should conceptually resemble systemd while remaining Mirage-native and capability-mediated.

## Production Acceptance Gate

Mirage must not report the final storage path online until all conditions below are true:

1. A PCI function matching `01/08/02` or AHCI `01/06/01` is discovered.
2. BAR is sized, validated as MMIO, mapped, and verified through page-table checks.
3. PCI Memory Space and Bus Master Enable are set and read back.
4. DMA queues/buffers are allocated by the DMA allocator, zeroed, page-aligned, contiguous, and capability-authorized.
5. Controller reset/enable completes through volatile MMIO with bounded timeouts.
6. Identify/readiness commands complete through real completion queues.
7. Namespaces/disks are parsed from hardware identify data.
8. Block layer validates and performs real read/write/flush.
9. QFS/ext4 mount reads a real superblock from the block device.
10. RootFS is mounted and published by the supervisor.
11. MTSS schedules Spider-rs as PID 1.
12. Spider-rs launches the first userspace process.

Until then, `Online` must be reserved for proven hardware paths only.
