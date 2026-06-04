# AMD Platform References

Mirage AMD platform implementation is based on public AMD references only. This file records the reference policy and the support gates used before a Ryzen or AMD platform path is treated as supportable.

Do not copy large specification text into Mirage documentation or source. Summarize requirements, cite the public document family, and encode only the behavior needed for Mirage mechanisms and supervisor policy.

## Implemented now

* The architecture rule is documented: Mirage uses public AMD references only.
* Current AMD support is structured around public document families rather than private SDKs, leaked documents, or reverse-engineered confidential material.
* The required reference categories are identified:
  * AMD64 Architecture Programmer's Manual for AMD64 instruction, privilege, paging, interrupt, and CPUID architecture.
  * Processor Programming References (PPRs) by AMD CPU family/model for processor-specific registers, feature behavior, topology details, and model-specific constraints.
  * AMD IOMMU specification for IOMMU discovery, command/event handling, device-table structures, translation controls, and interrupt-remapping concepts.
  * PCI vendor/device/class discovery for platform device identification, BAR ownership, class-code routing, and driver-service matching.
  * Public GPUOpen and AMDGPU references for AMD graphics discovery, display/GPU service boundaries, and publicly documented GPU programming concepts.
* Ryzen support is explicitly gated by CPUID vendor, family, model, stepping, PCI vendor/device IDs, feature bits, and PPR availability.

## Mocked now

* Mirage has typed CPU and platform descriptors, but the repository does not yet contain a complete public-reference matrix for every Ryzen family/model/stepping.
* PPR availability is treated as a policy gate, but there is not yet an automated tool that maps each discovered CPU to an approved PPR revision.
* PCI IDs and class codes are represented in platform and PCI code, but there is not yet a complete AMD chipset allowlist tied to service manifests.
* GPU references are acknowledged for future AMDGPU/display work; this document does not certify a full hardware GPU implementation.
* Reference verification is manual documentation policy today rather than a signed machine-readable database.

## Real hardware path

* Record CPUID vendor string and reject non-AMD vendors for AMD-specific Ryzen policy.
* Decode CPUID family, model, stepping, and feature bits before selecting any CPU-specific path.
* Match the decoded CPU tuple to a public PPR for that family/model and track the exact PPR revision used for implementation decisions.
* Enumerate PCI devices and gate service handoff on vendor ID, device ID, class code, BAR layout, interrupt capability, and public documentation coverage.
* For IOMMU support, require public AMD IOMMU specification coverage plus platform discovery evidence that the IOMMU unit and its resources are present.
* For graphics support, separate generic PCI discovery from AMDGPU/display service policy and use only public GPUOpen, AMDGPU, firmware-interface, and register references that are legally usable.
* Store reference decisions in boot logs or platform reports so a hardware enablement result can be audited after boot.

## Unsupported areas

* Private AMD documentation, leaked register descriptions, NDA-only headers, proprietary driver source without a compatible license, and copied specification tables are not acceptable inputs.
* A Ryzen brand name alone is not sufficient for support; the exact CPUID tuple and required public PPR must be available.
* A matching PCI vendor ID alone is not sufficient for support; device ID, class code, BAR layout, feature bits, and service policy must also match.
* Hardware behavior that requires opaque firmware calls or vendor-private side channels is unsupported unless it can be isolated behind a clearly documented, capability-restricted service contract.
* Large verbatim excerpts from AMD manuals, PPRs, IOMMU specifications, GPUOpen, or AMDGPU documentation must not be added to the repository.

## Next steps

* Add `docs/AMD_REFERENCE_MATRIX.md` with per-family/per-model PPR status, tested board references, and implementation notes.
* Add a machine-readable allowlist for supported AMD CPU tuples and chipset/IOMMU PCI IDs.
* Add CI checks that require a reference note when adding new AMD family/model, PCI ID, or feature-bit behavior.
* Add boot-report fields for CPUID vendor, family, model, stepping, feature bits, PCI IDs, class codes, and the chosen PPR/reference set.
* Add a documentation template for new Ryzen platform enablement that includes public reference names without copying protected text.
