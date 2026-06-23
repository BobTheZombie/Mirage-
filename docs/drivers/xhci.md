# Mirage xHCI Driver

Mirage's current xHCI bring-up is an early, bounded kernel path for controller discovery and liveness validation. It is original Rust code based on the xHCI specification, PCI behavior, and documented Linux-reference audit findings; no Linux code was copied.

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
- Root port status inspection and connected-port reset.

## Pending features

- Scratchpad allocation.
- Extended capability ownership handoff.
- Full Address Device and endpoint context construction.
- Transfer rings for control/interrupt/bulk endpoints.
- MSI/MSI-X ownership.
- IOMMU-aware DMA mapping.

## Test results

`cargo check` passed after the xHCI changes. Full ISO/QEMU validation is recorded in the PR/final report when available.
