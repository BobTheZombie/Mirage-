# AMD IOMMU Service

AMD IOMMU support protects DMA and device isolation in the Mirage platform model. The IOMMU path must stay capability-mediated because it controls which devices can access which memory.

## Architecture rule

Mirage must never rely on broad, default, identity-mapped DMA as the steady-state device model. Once IOMMU activation is implemented, no PCI device should receive unrestricted identity-mapped access to all physical memory by default. Every DMA-visible range must be derived from a supervisor-issued DMA capability, installed into a specific IOMMU domain, and removed when that capability is revoked or the owning service crashes.

Early boot may still require carefully bounded firmware or bootstrap exceptions before the supervisor has enough state to program isolation, but those exceptions must be documented as temporary bring-up mechanisms rather than normal driver authority.

## Implemented now

* Mirage has an `AmdIommu` software model for discovered IOMMU units. It tracks parsed PCI capability metadata, a mock device table, command buffer, event log, domains, and an explicit `translation_enabled` state that remains `false`.
* Mirage has an `AmdIommuCapability` parser for AMD IOMMU PCI capability-list records discovered through `mirage-pci` configuration-space snapshots.
* Mirage has `AmdIommuDeviceTable` and mock-encoded device-table entries for testing device-to-domain assignment without claiming a production hardware descriptor layout.
* Mirage has `AmdIommuCommandBuffer` with polling and bounded timeout handling for command completion.
* Mirage has `AmdIommuEventLog` for early tests and service integration before IRQ-backed event routing is implemented.
* Mirage has `AmdIommuDomain`, `AmdIommuMapping`, `DeviceId`, and `DmaAddress` descriptors for capability-checked DMA domain management.
* `discover_iommu_from_pci()` scans PCI function snapshots and returns parsed AMD IOMMU capabilities.
* `parse_iommu_capability()` parses mockable AMD IOMMU PCI capability records.
* `create_domain()`, `map_dma_region()`, `unmap_dma_region()`, and `assign_device_to_domain()` provide the initial supervisor-facing domain operations.
* `map_dma_region()` requires a Mirage DMA capability for the physical DMA buffer and returns a structured `DmaDenied` error when authority is missing, revoked, or insufficient.
* Mirage still has `AmdIommuId`, `IommuDeviceTable`, `AmdIommuResources`, and `AmdIommuHandoff` descriptors for supervisor service handoff.
* Capability validation for handoff resources requires authority over the IOMMU PCI device, MMIO range, device-table DMA buffer, and IRQ line.

## Deliberately not enabled yet

* Translation is not globally enabled by this crate.
* Device tables are modeled and mock-encoded, but not programmed as production hardware tables.
* Command and event buffers are modeled for polling, timeout behavior, and service integration tests, but not wired to real interrupt delivery.
* Real MMIO register layouts are gated behind the `hw-amd-iommu` Cargo feature and use explicit `#[repr(C)]` only for hardware ABI boundary structs.
* The current implementation does not claim complete AMD IOMMU register coverage, interrupt remapping support, ATS/PRI/PASID support, or virtualization/nested translation support.

## Real hardware path

* Discover AMD IOMMU units through public platform discovery mechanisms and PCI identification.
* Validate the discovered IOMMU against public AMD IOMMU documentation and Mirage supervisor policy.
* Allocate device tables, command buffers, event buffers, and page tables as supervisor-authorized DMA memory objects.
* Program IOMMU MMIO only from a service or kernel-adjacent component holding the required MMIO, PCI, IRQ, and DMA capabilities.
* Keep translation disabled until the supervisor has constructed per-device domains and safe mappings.
* Assign each device to a domain that contains only the DMA ranges explicitly authorized for its supervised driver service.
* Avoid broad identity maps. If a bootstrap identity map is unavoidable for a specific platform transition, constrain it to the minimum range, record why it exists, and remove it before normal driver operation.
* Route IOMMU events and faults through a capability-protected IRQ endpoint.
* On service crash, revoke DMA mappings, stop or quarantine affected devices, replay safe configuration, and restart according to supervisor policy.

## Unsupported areas

* Running devices with unrestricted DMA is unsupported for real hardware targets that require isolation.
* Giving ordinary applications raw IOMMU MMIO access is unsupported.
* Using undocumented IOMMU registers or copied proprietary tables is unsupported.
* Peer-to-peer DMA, ATS/PRI/PASID, nested translation, interrupt remapping, and virtualization-specific modes are not initial support targets unless explicitly documented and tested.
* A platform with an IOMMU that cannot be discovered or validated from public references is unsupported.

## Next steps

* Add an `iommud` service manifest with declared PCI, MMIO, IRQ, DMA, and IPC capabilities.
* Connect IOMMU device assignment to block, GPU, USB, and network service launch policy.
* Add crash cleanup that revokes domain mappings and replays only safe device assignments.
* Add boot-report fields for IOMMU units, device-table ranges, command/event buffers, and fault routing.
* Add a staged hardware bring-up checklist based on public AMD IOMMU references.
