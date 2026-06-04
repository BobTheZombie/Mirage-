# AMD Chipset Service

AMD chipset support in Mirage is modeled as a restartable supervised service whenever possible. The kernel enforces capabilities for PCI, MMIO, IRQ, DMA, and IPC resources; the supervisor decides whether a chipset service may run.

## Implemented now

* Mirage has an `AmdChipsetId` descriptor for Mirage-visible chipset identity.
* Mirage has an `AmdChipsetResources` descriptor containing PCI root, MMIO range, MMIO length, and IRQ line information.
* Capability validation requires authority over the PCI device, MMIO region, and IRQ line before a chipset handoff is valid.
* Mirage has an `AmdChipsetHandoff` record tying chipset identity, Ryzen profile, service endpoint, and resources together.
* The platform planner can create a restart-on-crash AMD chipset service launch request.

## Mocked now

* Chipset resource records are structured descriptors, not the result of a complete live PCI/ACPI discovery pipeline.
* No production `chipsetd` currently programs real AMD chipset registers.
* IRQ routing, BAR sizing, power-management coordination, and chipset-specific feature handling are not complete.
* Crash recovery is represented by supervisor restart policy and capability revocation expectations, not by a fully running hardware daemon.
* Device matching is not yet backed by a complete AMD chipset PCI ID allowlist.

## Real hardware path

* Enumerate PCI roots and chipset devices using PCI vendor/device/class discovery.
* Match AMD chipset devices to public references and board-specific discovery reports.
* Have the supervisor grant only the exact PCI, MMIO, IRQ, DMA, and IPC capabilities required by `chipsetd`.
* Run chipset control in a supervised driver service that can crash without forcing a kernel panic.
* On crash, revoke chipset service capabilities, reclaim resources, restart the service if policy allows, and restore registered IPC endpoints.
* Keep global policy such as power profile selection, service ordering, and recovery escalation in the supervisor.

## Unsupported areas

* Undocumented chipset register programming is unsupported.
* Direct application access to chipset PCI config space, MMIO, or IRQ lines is unsupported.
* Kernel-resident chipset policy is unsupported except for unavoidable early boot machine-control needs.
* Board-specific hacks without public documentation and explicit supervisor policy are unsupported.
* Features that require proprietary management engines or opaque firmware protocols are outside the initial target.

## Next steps

* Add a `chipsetd` service manifest with declared capabilities and restart behavior.
* Add PCI discovery integration that builds `AmdChipsetResources` from real enumerated devices.
* Add an AMD chipset PCI ID allowlist tied to public references.
* Add unit tests for successful and failed chipset capability validation.
* Add crash/restart demo documentation for chipset service recovery.
