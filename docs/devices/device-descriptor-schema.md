# Device Descriptor Schema

Device descriptor files are TOML source data consumed by host-side generators. The schema must be strict enough to reject ambiguous identities and rich enough to document provenance.

## Common fields

Each descriptor should include:

- `id`: stable Mirage identifier for cross-references.
- `bus`: `pci`, `usb`, `cpu`, `chipset`, `block`, `char`, or `input`.
- `name`: short human-readable device or family name.
- `match`: bus-specific matching keys.
- `driver_hints`: preferred driver module or service names.
- `quirks`: named mechanism flags used by drivers after validation.
- `capabilities`: capabilities the selected driver or service may need.
- `fallback`: safe behavior when the hinted driver cannot bind.
- `provenance`: source, license, import date, and notes.

## Driver hints

Driver hints are not launch authorization. They may name candidates such as `xhci`, `ahci`, `nvme`, `i8042`, `usb-hid-keyboard`, or supervised services such as `usbd`, `storaged`, and `inputd`. The Supervisor decides whether a service may launch and which capabilities it receives.

## Example

```toml
id = "pci-qemu-xhci"
bus = "pci"
name = "QEMU xHCI controller"
driver_hints = ["xhci", "usbd"]
quirks = []
capabilities = ["pci.device", "irq.line", "dma.region"]
fallback = "disable-device-with-diagnostic"

[match.pci]
vendor_id = 0x1b36
device_id = 0x000d
class = 0x0c
subclass = 0x03
prog_if = 0x30

[provenance]
source = "QEMU public device documentation and observed QEMU enumeration"
license = "documentation/observation; no copied implementation code"
imported_on = "2026-06-27"
notes = "Identifiers require validation against generated-table tests before use."
```

## Validation expectations

Validation must reject duplicate stable IDs, malformed numeric IDs, missing provenance, unknown quirk names, unknown fallback policies, and descriptors that mix incompatible bus match fields.
