# Mirage USB Driver Notes

The early USB stack is split into staged modules: `xhci-host0`, `usb-core0`, `usb-hid0`, and `usb-kbd0`. The stack must not block boot and must not mark a device online until the relevant hardware path has executed.

## Enumeration flow

1. Bring xHCI online with command/event validation.
2. Read root hub port count from xHCI structural parameters.
3. Inspect every PORTSC register.
4. Reset connected ports with bounded waits.
5. Record speed and port identity.
6. Future work: Enable Slot, Address Device, read device/configuration/interface/endpoint descriptors, and configure class drivers.

## HID boot keyboard

The input decoder for HID boot reports exists, but `USB KEYBOARD` is marked online only after a HID boot keyboard is actually configured. The stack never waits for a first key event during boot.

## Mass storage

Mass-storage detection and BOT/SCSI abstractions exist in `mirage-usb`; the early kernel xHCI path does not yet claim storage online because descriptor reads and bulk transfer rings are pending.

## Provenance

Linux USB files listed in `docs/audits/linux-xhci-amd-renoir-reference-audit.md` were inspected for behavior and quirk awareness. Mirage implementation is independent Rust and does not copy Linux code.
