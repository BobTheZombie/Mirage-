# AMD IOMMU Service

AMD IOMMU support protects DMA and device isolation in the Mirage platform model. The IOMMU path must stay capability-mediated because it controls which devices can access which memory.

## Implemented now

* Mirage has an `AmdIommuId` descriptor for Mirage-visible IOMMU identity.
* Mirage has an `IommuDeviceTable` descriptor for the device-table memory range owned by an IOMMU instance.
* Mirage has `AmdIommuResources` for PCI device identity, MMIO range, IRQ line, and device-table DMA buffer information.
* Capability validation requires authority over the IOMMU PCI device, MMIO region, device-table DMA buffer, and IRQ line.
* Mirage has an `AmdIommuHandoff` record tying IOMMU identity, Ryzen profile, service endpoint, and resources together.
* The platform planner can create a restart-on-crash AMD IOMMU service launch request.

## Mocked now

* Device-table allocation and command/event ring programming are represented as descriptors, not production hardware programming.
* There is no complete AMD IOMMU interrupt handler, fault logger, command processor, or device-isolation policy database yet.
* PCI requester-ID mapping and per-device DMA domains are not fully wired to the block, GPU, USB, or network service stack.
* IOMMU service recovery is described by the supervisor model, but hardware quiesce/replay logic is not complete.
* The implementation does not yet parse every firmware discovery source needed on physical systems.

## Real hardware path

* Discover AMD IOMMU units through public platform discovery mechanisms and PCI identification.
* Validate the discovered IOMMU against the public AMD IOMMU specification and processor/platform references.
* Allocate device tables, command buffers, event buffers, and page tables as supervisor-authorized DMA memory objects.
* Program IOMMU MMIO only from a service or kernel-adjacent component holding the required MMIO and PCI capabilities.
* Route IOMMU events and faults through a capability-protected IRQ endpoint.
* Integrate device assignment so each supervised driver service receives only DMA authority for the memory objects it owns.
* On service crash, revoke DMA mappings, stop or quarantine affected devices, replay safe configuration, and restart according to supervisor policy.

## Unsupported areas

* Running devices with unrestricted DMA is unsupported for real hardware targets that require isolation.
* Using undocumented IOMMU registers or copied proprietary tables is unsupported.
* Passing raw IOMMU MMIO access to ordinary applications is unsupported.
* Peer-to-peer DMA, ATS/PRI/PASID, nested translation, interrupt remapping, and virtualization-specific modes are not initial support targets unless explicitly documented and tested.
* A platform with an IOMMU that cannot be discovered or validated from public references is unsupported.

## Next steps

* Add an `iommud` service manifest with declared PCI, MMIO, IRQ, DMA, and IPC capabilities.
* Add device-domain data structures for supervised services.
* Add mock tests for DMA mapping grant, revoke, and crash cleanup.
* Add boot-report fields for IOMMU units, device-table ranges, command/event buffers, and fault routing.
* Add a staged hardware bring-up checklist based on the public AMD IOMMU specification.
