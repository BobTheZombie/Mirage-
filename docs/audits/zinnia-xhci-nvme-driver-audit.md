# Zinnia xHCI/NVMe Driver Audit

## Zinnia commit inspected

- Repository: `https://github.com/zinnia-os/zinnia`
- Commit inspected: `ecbedd86ab8fe70a5db02eabcf35966b77f0eb56`
- Local audit checkout: `/tmp/zinnia-driver-audit`

## Zinnia license summary

Zinnia's top-level `LICENSE` is GPL-2.0 text. Mirage is not importing Zinnia source code in this patch. Zinnia was used only as an external behavioral reference for device-driver architecture, queue sequencing, and timeout/probe patterns. No Zinnia code was copied into Mirage.

## Zinnia xHCI/NVMe files inspected

NVMe:

- `drivers/block/nvme/Cargo.toml`
- `drivers/block/nvme/src/lib.rs`
- `drivers/block/nvme/src/controller.rs`
- `drivers/block/nvme/src/queue.rs`
- `drivers/block/nvme/src/command.rs`
- `drivers/block/nvme/src/spec.rs`
- `drivers/block/nvme/src/namespace.rs`
- `drivers/block/nvme/src/error.rs`

xHCI:

- `drivers/usb/xhci/Cargo.toml`
- `drivers/usb/xhci/src/lib.rs`
- `drivers/usb/xhci/src/device.rs`
- `drivers/usb/xhci/src/hub.rs`
- `drivers/usb/xhci/src/ring.rs`
- `drivers/usb/xhci/src/spec.rs`
- `drivers/usb/xhci/src/transfer.rs`

## Mirage driver files inspected

- `crates/mirage-pci/src/lib.rs`
- `crates/mirage-nvme/src/lib.rs`
- `crates/mirage-usb/src/lib.rs`
- `crates/mirage-platform/src/lib.rs`
- `src/kernel/mmio.rs`
- `src/kernel/device.rs`
- `docs/drivers/xhci.md`
- `docs/drivers/usb.md`
- `docs/storage/nvme.md`
- `docs/storage/ahci.md`

## What was learned

- NVMe bring-up should be staged: PCI class match, BAR validation, controller disable, admin queue allocation, AQA/ASQ/ACQ programming, controller enable, identify controller, identify namespaces, then register block devices.
- xHCI bring-up should be staged: PCI class match, BAR validation, capability register parsing, operational/runtime/doorbell offset derivation, halt/reset, ring allocation, interrupter/event-ring setup, run, then root-hub/port handling.
- Optional driver failures must be nonfatal and must report the last real step reached.
- Polling paths are useful during early bring-up, but every poll must be bounded and carry an operation name for diagnostics.
- MSI/MSI-X integration is useful later, but polling fallback is acceptable for initial controller liveness if explicitly degraded and bounded.

## What was independently reimplemented

This patch adds shared Mirage-native PCI driver scaffolding in `crates/mirage-pci/src/lib.rs`:

- `DriverProbeResult` for Bound/NotSupported/Degraded/Failed probe results.
- `DriverError` for common early hardware failure categories.
- `Timeout`, `poll_until`, and `poll_reg_until` for bounded polling.
- `DmaBufferDescriptor` for alignment-checked DMA descriptors.
- `require_mmio_bar` for common MMIO BAR validation.
- Unit tests for bounded polling, DMA alignment, MMIO BAR validation, PCI class matching, and BAR decoding.

Existing Mirage NVMe/xHCI crates already contain Mirage-native mock/hardware-gated scaffolding. This audit documents that their status must remain honest: no namespace/block device is online before identify succeeds, and no USB root hub is online before xHCI init/ring/event liveness has actually succeeded.

## Whether any code was copied

No Zinnia source code was copied. The implementation is original Mirage Rust based on PCI/NVMe/xHCI concepts and the staged architecture observed during audit.

## License/provenance note

Because Zinnia is GPL-2.0, this audit treats it as a reference only. Any future direct code import would require explicit license review, file-level provenance, and an architectural decision that the imported code is compatible with Mirage's licensing and design. This patch does not make that import.

## Remaining gaps

- Wire the new shared `DriverProbeResult` through all kernel hardware driver startup logs.
- Replace remaining driver-local polling helpers with shared `poll_until`/`poll_reg_until` where crate boundaries allow it.
- Add real target MMIO volatile access adapters around `src/kernel/mmio.rs` for NVMe/xHCI hardware paths.
- Complete target NVMe identify/admin/read queues against real DMA allocation and physical-address lookup.
- Complete target xHCI root-hub port reset, slot enable/address-device flow, HID keyboard polling, and optional USB mass storage.
- Add MSI/MSI-X capability parsing and IRQ registration once interrupt ownership policy is ready.
