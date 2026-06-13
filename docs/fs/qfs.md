# QFS on Block Devices

QFS mounts from the Mirage block abstraction. Mounting reads the superblock, validates magic/version/sector size, loads root metadata, and supports path lookup and file reads through bounded block operations.

Root selection policy:

1. `root=nvme0n1` tries the NVMe namespace by name.
2. `root=sata0` tries the SATA disk by name.
3. `root=builtin-qfs` uses only the explicit built-in fallback.
4. `root=auto` tries `nvme0n1`, then `sata0`, then `BuiltInBlockQfs` only if configured.

Root probing must not write to disk.
