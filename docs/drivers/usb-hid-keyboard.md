# USB HID Keyboard

USB HID keyboard support is an optional input source layered above xHCI. PS/2 keyboard support remains the baseline input path and must not regress when USB is absent or failed.

The USB keyboard path must enumerate a real HID keyboard interface, set configuration/boot protocol when supported, poll the interrupt-IN endpoint without blocking boot, and translate key events into Mirage's bounded input queue. Absence of a keyboard is `DEGRADED: no HID keyboard found`, not a boot failure.
