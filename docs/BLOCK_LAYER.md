# Mirage Block Layer

The Mirage block layer is the narrow mechanism boundary between storage-facing
services and higher-level filesystems such as QFS. It intentionally describes
logical block devices, validated ranges, request ordering, and completion
reporting without encoding USB, NVMe, AHCI, QFS, POSIX, or Linux policy.

Mirage storage should be read as this layering:

```text
QFS / filesystem services
    -> storage handle or generic block device capability
    -> Mirage block layer request model
    -> supervised storage driver service or signed loadable module
    -> capability-checked hardware resources
```

## Responsibilities

The block layer owns backend-independent storage mechanics:

* stable block device identifiers;
* logical block sizes and sector counts;
* LBA-based block ranges;
* request identifiers;
* read, write, and flush request descriptions;
* static device information such as read-only and write-cache state;
* runtime online, offline, and faulted device state;
* buffer length and range validation;
* simple queueing and completion records;
* a backend-neutral `BlockDevice` trait for mocks, services, and modules.

The block layer is therefore suitable for unit tests and service boundary tests:
a RAM-backed mock, a USB mass-storage service, an NVMe module, and an AHCI
service should all be able to expose the same generic block interface.

## Non-responsibilities

The block layer must not become a storage policy or hardware driver subsystem.
It does not own:

* PCI or USB enumeration;
* MMIO mapping policy;
* DMA allocation policy;
* IRQ routing policy;
* queue-depth tuning policy;
* disk scheduling policy beyond simple mockable request ordering;
* partition-mount policy;
* QFS object semantics;
* POSIX file-descriptor semantics;
* Linux block-device ABI compatibility;
* filesystem journaling or recovery policy;
* cryptographic signature validation policy.

Those decisions belong to the supervisor, storage services, filesystem services,
or signed driver modules. The kernel enforces capabilities and provides low-level
mechanisms; the supervisor decides which service receives which authority.

## Capability boundary

A block request is only valid when it is submitted through a storage handle or
IPC endpoint backed by an appropriate capability. The minimum authority model is:

| Operation | Required authority |
| --- | --- |
| Read | read access to the storage handle or block device |
| Write | write access to the same device and a non-read-only device state |
| Flush | flush/cache-management access to the same device |
| Hotplug registration | supervisor-granted service registration authority |
| Device removal | supervisor or owning driver-service authority |

Capability checks should happen before a request is dispatched to driver code.
Drivers must not infer authority from process identity, device names, Unix paths,
or Linux major/minor numbers.

## QFS dependency rule

QFS depends only on a generic block device or a supervisor-issued storage handle.
It must not depend on whether bytes came from USB mass storage, NVMe, AHCI, a
RAM disk, or a bootloader-provided mock device.

This rule keeps QFS aligned with Mirage's native object model:

* QFS owns object IDs, path identity, metadata, extent maps, journal state, and
  optional signatures.
* The storage layer owns durable block reads, writes, flushes, and hotplug state.
* Driver services own protocol-specific transport details.
* The supervisor owns policy for granting QFS access to particular storage
  handles.

As a result, a QFS root can move from a mock RAM block device to USB, NVMe, or
AHCI without changing QFS object logic.

## Mock-first development standard

Early Mirage storage work should prefer explicit mocks over fake completeness.
A driver or storage backend may advertise mock status, fixed capacity, simplified
partition metadata, or non-production persistence as long as those limitations
are visible in the API and documentation. It is better to prove the capability,
IPC, restart, and QFS lookup boundaries than to pretend full hardware support
exists.
