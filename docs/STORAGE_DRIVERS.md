# Mirage Storage Drivers

Mirage storage drivers are protocol adapters that turn hardware-specific storage
operations into generic block devices or storage handles. They are not supposed
to expose Linux block APIs, Linux device names, or filesystem policy to QFS.

The preferred driver form is a supervised service. A driver may instead be a
signed loadable kernel module when early boot constraints or performance-critical
kernel-adjacent work require it, but that is an exception rather than the normal
model.

## Common driver responsibilities

USB mass-storage, NVMe, and AHCI implementations should provide the same service
shape:

* discover devices only when the supervisor grants enumeration authority;
* claim hardware only with explicit device capabilities;
* map controller registers only with MMIO capabilities;
* allocate and use DMA only inside granted DMA regions;
* receive interrupts only for granted IRQ lines or MSI/MSI-X vectors;
* translate protocol-specific commands into generic block reads, writes, and
  flushes;
* register block devices with the supervisor-controlled storage registry;
* surface hotplug, removal, media-change, offline, and fault events;
* revoke or drop device capabilities on crash and restart;
* remain mockable with deterministic in-memory devices for early tests.

## Capability requirements

The supervisor must grant scoped capabilities before any storage driver touches
hardware. The exact object identifiers are platform-specific, but the authority
classes should be explicit.

| Driver | Required capabilities |
| --- | --- |
| USB storage (`usbd` or USB storage module) | USB controller or device claim capability, endpoint/pipe authority, DMA region, IRQ/MSI routing, storage-service registration endpoint, optional removable-media notification endpoint |
| NVMe (`nvmed` or signed module) | PCI device claim capability, BAR/MMIO mapping, admin queue DMA, I/O queue DMA, MSI/MSI-X vector authority, controller reset authority, storage-service registration endpoint |
| AHCI (`ahcid` or signed module) | PCI device claim capability, AHCI BAR/MMIO mapping, command-list/FIS DMA region, port ownership capability, IRQ/MSI authority, controller reset authority, storage-service registration endpoint |

A driver must not receive broad kernel memory access or unrestricted PCI/USB
access. Capabilities should be revocable so that the supervisor can recover from
a crashed or wedged service.

## USB storage role

The USB storage role is to own USB transport details, not filesystem details.
A supervised `usbd` stack may expose a mass-storage child service, or a dedicated
USB-storage service may receive a device claim from `usbd`. Either way, the
storage-facing output is a generic block device.

Mockable responsibilities include:

* fake device attach and detach events;
* endpoint selection;
* command block wrapper / transport placeholders;
* fixed-size RAM-backed media;
* read/write/flush translation into the block layer.

Non-goals for the early scaffold include complete USB enumeration, xHCI
production support, UASP, power management, and all removable-media edge cases.

## NVMe role

The NVMe role is to own controller and namespace mechanics. It should enumerate
namespaces and expose each usable namespace as a generic block device or storage
handle.

Mockable responsibilities include:

* synthetic PCI identity;
* simplified controller initialization state;
* mock admin and I/O queue setup;
* namespace capacity reporting;
* read/write/flush command translation.

Non-goals for the early scaffold include production queue tuning, real PRP/SGL
management, multipath, namespaces with advanced metadata, live firmware update,
and complete error recovery.

## AHCI role

The AHCI role is to own SATA controller, port, and command-list mechanics. It
should expose each attached SATA disk as a generic block device or storage
handle.

Mockable responsibilities include:

* synthetic PCI identity;
* fixed AHCI port inventory;
* mock command header and FIS lifecycle;
* read/write/flush translation;
* fault injection for removed or failed media.

Non-goals for the early scaffold include full SATA link power management,
complete ATAPI support, NCQ tuning, hotplug edge-case coverage, and production
error recovery.

## Supervised service versus loadable module

A supervised driver service is preferred when the driver can run outside kernel
privilege and talk over IPC. This allows the supervisor to detect crashes,
revoke capabilities, reclaim resources, restart the driver, and restore service
endpoints.

A signed loadable module may be used when a component must be kernel-adjacent.
Even then, the module should expose the same generic block semantics upward and
must not bypass capability enforcement. Module loading itself is policy owned by
the supervisor and enforced by signed-module validation.

## Relationship to QFS

QFS receives a storage handle or generic block device capability. It does not
call USB, NVMe, or AHCI driver internals. This keeps QFS reusable across boot
media, test media, removable media, and future storage transports while
preserving the Mirage rule: Unix-compatible outside, capability-secured Mirage
mechanisms inside.
