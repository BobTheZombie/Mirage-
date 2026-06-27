# USB Device Database

The USB database records VID/PID, device class, interface class, subclass, protocol, and HID usage hints for USB driver selection.

## Required fields

USB descriptors should include either exact `vendor_id` and `product_id` matches or class/interface matches. Interface descriptors must include interface class, subclass, and protocol when those values are required for safe binding.

## Driver hints and quirks

USB hints may reference `xhci`, `usbd`, `usb-hid-keyboard`, `inputd`, storage services, or future audio/network services. Hints do not prove device readiness. xHCI and USB drivers must still validate descriptors, endpoints, transfer rings, and event completion before status advances.

## Fallback behavior

Unknown USB devices should be ignored, exposed as unsupported, or matched by a safe class driver. USB input must never block boot waiting for the first key event, and unknown HID usages must not panic.

## Provenance

VID/PID names and class mappings must record provenance and license. Locally observed descriptors may be documented as observation-derived data when no external database is imported.
