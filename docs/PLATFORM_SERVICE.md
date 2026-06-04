# Platform Service Model

The Mirage platform service model keeps hardware policy out of the kernel. Platform services are supervised, capability-limited components that translate discovered hardware facts into controlled driver and runtime behavior.

## Implemented now

* Mirage defines platform service kinds for AMD chipset, AMD IOMMU, and AMD telemetry.
* Mirage defines restart policies for restart-on-crash and manual recovery.
* Mirage defines generic service launch requests containing service kind, IPC endpoint, and restart behavior.
* Mirage defines supervisor handoff records containing launch requests plus a capability set.
* Mirage can validate that a handoff contains the IPC endpoint capability needed by the target service.
* The AMD platform policy planner creates launch requests for chipset, IOMMU, and telemetry services without performing raw hardware access.

## Mocked now

* Service launch requests are plain records; the full supervisor daemon that consumes them is not complete.
* Endpoint registration, service discovery, crash detection, and endpoint restoration are architectural requirements, not a complete production runtime here.
* Hardware discovery is not yet automatically connected to all platform service manifests.
* Capability bundles can be validated by helper methods, but no full boot policy engine currently grants every platform service from a signed manifest.
* Telemetry, chipset, and IOMMU service binaries are not fully implemented production daemons.

## Real hardware path

* Boot discovery produces a platform report containing CPU, firmware, PCI, MMIO, IRQ, DMA, and feature-bit facts.
* The supervisor matches that report against signed service manifests and public hardware references.
* The supervisor grants each platform service only the capabilities listed in its manifest and validated by the discovered hardware.
* Platform services register IPC endpoints through supervisor-controlled service discovery.
* Driver services communicate through IPC and shared memory rather than global kernel calls.
* On service crash, the supervisor detects failure, revokes capabilities, reclaims resources, restarts or quarantines the service, and restores IPC endpoints according to policy.
* Kernel code remains mechanism-only: scheduling, address spaces, interrupts, IPC transport, and capability enforcement.

## Unsupported areas

* Platform services must not bypass capabilities with raw port, MMIO, DMA, IRQ, MSR, or PCI config access.
* A platform service must not become a hidden monolithic kernel driver through unchecked kernel callbacks.
* Unsigned modules or services are unsupported for privileged hardware access.
* Device support without a manifest, reference note, and capability plan is unsupported.
* Linux-specific assumptions such as `/sys`, `/proc`, udev rules, or kernel driver binding are compatibility-layer concerns, not native Mirage platform-service mechanisms.

## Next steps

* Add signed service manifests for AMD chipset, AMD IOMMU, AMD telemetry, storage, USB, networking, and display services.
* Add a platform discovery report format shared by boot code, the supervisor, and service policy.
* Add a supervisor policy engine that turns reports and manifests into capability grants.
* Add service crash/restart tests that verify capability revocation and endpoint restoration.
* Add documentation linking each platform service to its required hardware references and unsupported fallback behavior.
