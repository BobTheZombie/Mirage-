# Mirage Root Filesystem Policy

Supported root selectors:

- `root=auto`
- `root=builtin-qfs`
- `root=nvme0n1`
- `root=sata0`

Expected logs:

```text
[rootfs] trying nvme0n1
[rootfs] mounted QFS on nvme0n1
[rootfs] trying sata0
[rootfs] mounted QFS on sata0
[rootfs] trying builtin-qfs
[rootfs] mounted BuiltInBlockQfs
```

`Online` is reserved for successful real mounts. Missing hardware is `Skipped`; detected-but-unusable hardware is `Failed`.

## Spider Runtime and Device Root Selection

Root selection accepts whole devices and partitions: `root=sata0`, `root=sata0p1`, `root=nvme0n1`, `root=nvme0n1p1`, `root=atapi0`, `root=qfs:sata0p1`, `root=ext4:sata0p1`, and `root=auto`.

The updated auto policy mounts Spider Runtime first, launches Spider-rs from `/spider-rt`, enumerates block devices, parses partitions, then attempts explicit or configured QFS/ext4 root mounting while keeping Spider Runtime available for recovery.
