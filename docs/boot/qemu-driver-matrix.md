# QEMU Driver Matrix

Normal boot keeps the existing AHCI/i8042 path and should reach the current boot milestones.

NVMe test hardware can be enabled with a guarded QEMU extra-args path:

```bash
qemu-system-x86_64 \
  -drive if=none,id=nvme0,file=build/nvme-test.img,format=raw \
  -device nvme,drive=nvme0,serial=mirage-nvme0
```

xHCI and USB keyboard test hardware can be enabled with:

```bash
qemu-system-x86_64 \
  -device qemu-xhci,id=xhci \
  -device usb-kbd,bus=xhci.0
```

Runner integrations should remain environment-guarded, such as `MIRAGE_QEMU_NVME=1`, `MIRAGE_QEMU_XHCI=1`, and `MIRAGE_QEMU_USB_KBD=1`, or existing `MIRAGE_QEMU_EXTRA_ARGS` flows. Optional NVMe/xHCI failures must produce exact nonfatal status rather than silent hangs.
