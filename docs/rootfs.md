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
