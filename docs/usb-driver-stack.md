# Mirage USB Driver Stack

Mirage USB input is now represented as a modular driver stack rather than a single inline boot-time HID initializer:

```text
PCI
  -> xhci-host0
  -> usb-core0
  -> usb-hid0
  -> usb-kbd0
```

The modules are kernel-registered early driver modules today, with descriptors, dependencies, lifecycle entry points, status, and fixed-capacity state.  Their boundaries are intentionally aligned with the future supervisor-owned driver-service model, where PCI/MMIO/DMA/IRQ authority can be granted and revoked through capabilities.

## Modules

### `xhci-host0`

`xhci-host0` owns xHCI host-controller discovery and bring-up:

- scans PCI config space for class `0x0c`, subclass `0x03`, programming interface `0x30`;
- enables PCI memory space and bus mastering;
- maps BAR0 through Mirage's current HHDM-aware boot path;
- parses xHCI capability registers, `HCSPARAMS1`, `HCCPARAMS1`, `DBOFF`, and `RTSOFF`;
- halts and resets the controller using bounded waits;
- prepares fixed static DCBAA, command-ring, event-ring, and ERST backing;
- configures max slots and runs the controller.

It does **not** enumerate HID keyboards directly.

### `usb-core0`

`usb-core0` depends on `xhci-host0`.  It owns the bus-manager role:

- scans root ports;
- resets connected ports using bounded waits;
- assigns bounded Mirage USB device records;
- exposes fixed-capacity device metadata to class drivers.

Current limitation: the early kernel path does not yet submit the full Enable Slot / Address Device / descriptor control-transfer sequence through a production DMA command path.  The module boundary and timeout-safe port handling are in place so that command-ring enumeration can be completed without reintroducing inline boot HID code.

### `usb-hid0`

`usb-hid0` depends on `usb-core0`.  It scans USB core device records and creates fixed-capacity HID records for boot-protocol keyboard candidates.  The descriptor parser rules documented for Mirage remain strict: descriptor lengths must be bounded, descriptor iteration must make forward progress, malformed descriptors must return a typed error, and HID class control requests must time out rather than block boot.

### `usb-kbd0`

`usb-kbd0` depends on `usb-hid0`.  It binds to boot keyboard HID records, configures the interrupt-IN endpoint record, marks the USB HID input source online, and uses the same Mirage keyboard event queue as PS/2.  It is online once endpoint scheduling is configured; it never waits for a user keypress during init.

## Boot Phase Manager status

The boot phase screen and serial log now track:

```text
xHCI       [ ONLINE / SKIPPED / FAILED ]
USB Core   [ ONLINE / SKIPPED / FAILED ]
USB HID    [ ONLINE / SKIPPED / FAILED ]
USB Kbd    [ ONLINE / SKIPPED / FAILED ]
```

Dependency skips propagate downward.  For example, if there is no xHCI controller, `usb-core0`, `usb-hid0`, and `usb-kbd0` skip rather than blocking boot.

## No-hang policy

USB modules are optional early drivers.  Every hardware wait in this path is bounded:

- controller halt;
- controller reset;
- controller run;
- root-port reset;
- root-port enable.

Future command-ring, transfer-ring, descriptor, HID request, and interrupt-transfer waits must follow the same rule: no infinite spin and no boot stop for optional USB failure.

## QEMU targets

```sh
make qemu-usb-none
make qemu-usb-kbd
make qemu-keyboard-all
```

Expected behavior:

- `qemu-usb-none`: USB modules skip cleanly if no xHCI controller/device is exposed.
- `qemu-usb-kbd`: QEMU starts with `-device qemu-xhci -device usb-kbd`.
- `qemu-keyboard-all`: PS/2 and USB keyboard paths are both enabled; PS/2 must remain independent.

## Future supervised migration

The current modules are kernel-registered because the external supervised driver-module ABI is still maturing.  The intended migration is:

1. supervisor grants PCI/MMIO/DMA/IRQ capabilities to `xhci-host0`;
2. USB core becomes a supervised bus manager service;
3. HID and keyboard class drivers become restartable child services;
4. crash recovery revokes endpoints/capabilities and restarts failed services without panicking the kernel.
