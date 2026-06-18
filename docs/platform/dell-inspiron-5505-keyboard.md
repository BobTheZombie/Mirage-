# Dell Inspiron 15 5505 FHD Internal Keyboard

The Dell Inspiron 15 5505 FHD / AMD Ryzen 5 4500U platform exposes its internal keyboard through the classic i8042 controller and AT/PS2 keyboard protocol. This path is equivalent in shape to Linux `i8042 + serio + atkbd`, but Mirage keeps the lower-kernel and supervisor responsibilities separate.

## Internal PS/2 versus external USB

The internal keyboard is not USB HID. It is reached through i8042 data port `0x60`, command/status port `0x64`, and IRQ1. External keyboards continue to use the USB xHCI/HID stack and publish into the same structured input queue after their own hardware-backed decode path.

## Expected boot path

On working hardware Mirage should report `I8042 [Detected]`, `I8042 [Started]`, `I8042 [Ok]`, `PS/2 Keyboard [Started]`, and only later `PS/2 Keyboard [Online]` after the first Set 2 event is decoded. If the controller is absent, the boot path should skip i8042 and PS/2 keyboard without panicking.

## Debug shell

The ESC key is routed through the shared input queue to the early debug shell hotkey path. Inside the shell, typed characters, Backspace, and Enter are consumed from structured keyboard events. The `input`, `keyboard`, or `kbdstat` shell command prints status, mode, scan set, event counts, overflow count, and last key.

## Touchpad next step

The laptop touchpad is not expected to be part of the keyboard's i8042 path. Future touchpad support should audit ACPI, I2C-HID, and possibly GPIO interrupt routing independently.
