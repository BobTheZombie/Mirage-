# Renoir xHCI and PID1 audit note

## Boundary audit

* Lower-kernel xHCI code performs PCI command enable, BAR lookup, volatile MMIO register access, controller reset/start, static ring setup, and polled root-port reset.
* Supervisor USB records only track policy state, status reasons, service visibility, HID routing approval, and a single reset retry. They do not write MMIO registers, enqueue TRBs, or mutate MTSS queues.
* Spider-rs PID1 launch flows through RuntimeVfs -> supervisor authorization -> kernel ELF validation -> kernel/MTSS process admission.

## Status audit

* Renoir CPU / topology / SoC detection is hardware-backed by CPUID facts.
* AMD xHCI detection is hardware-backed by CPUID/platform and PCI class matching.
* xHCI `STARTED` is hardware-backed by controller reset/start. `ONLINE` is reserved for a command/event completion path and must not be emitted by reset/start alone.
* AMDGPU Renoir and AMD IOMMU remain `STUB`/`SKIPPED` unless a future patch adds complete safe initialization.

## Remaining blockers

* Replace static early DMA buffers with allocator-backed physical DMA objects before enabling broad transfer traffic.
* Complete xHCI command/event smoke test and command completion parsing before allowing `ONLINE`.
* Complete Address Device / descriptor / Set Configuration transfer path before advertising HID keyboard online.
* Complete architecture ring-3 entry confirmation before marking userspace online.
