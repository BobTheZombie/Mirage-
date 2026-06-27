# Mirage xHCI Driver

Mirage's current xHCI bring-up is an early, bounded kernel path for controller discovery and liveness validation. Today it proves only the PCI/MMIO/reset/ring/No-Op command-completion path: a controller can be found, mapped, reset, given command/event rings, started, and observed completing a No-Op command. It is original Rust code based on the xHCI specification, PCI behavior, and documented Linux-reference audit findings; Linux source was inspected only for behavior/provenance, and no GPL code was copied.

## Init order

1. Select PCI device with class `0x0c`, subclass `0x03`, prog-if `0x30`.
2. Validate BAR0 as MMIO and enable PCI memory-space plus bus-master bits.
3. Map MMIO through the boot HHDM path.
4. Validate xHCI capability length, structural parameters, runtime offset, and doorbell offset.
5. Halt the controller with a bounded wait.
6. Reset the controller with a bounded wait.
7. Reject advertised scratchpads until Mirage has a DMA-safe scratchpad allocator.
8. Configure DCBAA, command ring, event ring, ERST, and interrupter 0.
9. Program `CONFIG`, start the controller, and wait for running state.
10. Submit a No-Op command and poll for a command-completion event before reporting `ONLINE`.

## MMIO overview

- Capability registers: `CAPLENGTH`, `HCSPARAMS1`, `HCSPARAMS2`, `HCCPARAMS1`, `DBOFF`, `RTSOFF`.
- Operational registers: `USBCMD`, `USBSTS`, `CRCR`, `DCBAAP`, `CONFIG`, `PORTSC[n]`.
- Runtime registers: interrupter 0 `IMAN`, `IMOD`, `ERSTSZ`, `ERSTBA`, `ERDP`.
- Doorbells: doorbell 0 is used for command-ring notification.

## Rings

The early path uses fixed aligned memory for DCBAA, command ring, event ring, and ERST. Addresses are translated to physical addresses before programming the controller. The command ring includes a link TRB and the event ring is acknowledged through ERDP.

## Implemented subset

- PCI detection and MMIO binding.
- Bounded reset/start.
- Command/event No-Op liveness proof.
- Polling mode fallback; MSI/MSI-X is not required for boot.
- Root port status inspection and bounded connected-port reset.
- No USB Address Device or descriptor enumeration is completed yet; connected ports are reset only.

## Pending features

- Scratchpad buffer allocation when advertised by the controller.
- BIOS/OS ownership handoff through xHCI extended capabilities.
- IOMMU-aware DMA mapping and ownership.
- MSI/MSI-X interrupt ownership; polling remains the early fallback.
- Full Enable Slot / Address Device flow and endpoint context construction.
- Device, configuration, interface, endpoint, HID, and storage descriptor reads.
- Transfer rings for control, interrupt, and bulk endpoints.

## Test results

`cargo check` passed after the xHCI changes. Full ISO/QEMU validation is recorded in the PR/final report when available.

## Zinnia reference audit

`docs/audits/zinnia-xhci-nvme-driver-audit.md` records the Zinnia xHCI files inspected at commit `ecbedd86ab8fe70a5db02eabcf35966b77f0eb56`. Zinnia is GPL-2.0, so Mirage uses it only as a behavioral reference. No Zinnia code was copied.
