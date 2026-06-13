# USB HID Boot Keyboard

USB HID boot keyboard support is split into `usb-hid0` and `usb-kbd0`.

A real keyboard must be identified by descriptors:

- interface class `0x03`
- subclass `0x01`
- protocol `0x01`
- interrupt-IN endpoint with at least an 8-byte max packet

The keyboard report format is the USB boot report: modifier byte, reserved byte,
and six key usage bytes. Mirage diffs previous/current reports and publishes the
same common keyboard events used by PS/2. ESC maps to the debug-shell hotkey path.

Current limitation: endpoint polling is not armed until xHCI control transfers and
endpoint configuration are complete, so HID/keyboard phases skip honestly when no
real descriptor-backed binding exists.
