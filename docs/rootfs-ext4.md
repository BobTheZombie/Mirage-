# ext4 root filesystem

Mirage's intended ext4 root path is:

```text
AHCI/NVMe/M.2 hardware -> BlockStorageDevice -> VFS -> ext4 -> /sbin/spider-rs
```

## QEMU image

```sh
make ext4-image
```

This creates `build/ext4-root.img`, formats it as non-journaled ext4, and installs:

- `/sbin/spider-rs`
- `/etc/spider/system/default.target`
- `/etc/spider/system/basic.target`
- `/bin/hello`

## QEMU boot targets

```sh
make qemu-ahci-ext4
make qemu-nvme-ext4
```

The targets attach the image through AHCI or NVMe and pass `root=ext4:sata0` or `root=ext4:nvme0n1`. The current kernel still needs full command-line root binding; discovered block devices try ext4 before QFS during automatic mount selection.

## Safety

- ext4 is read-only by default.
- `rootflags=rw` is documented for the future but not enabled for unsafe ext4 writes.
- Journaled ext4 write mode is refused until JBD2 exists.
