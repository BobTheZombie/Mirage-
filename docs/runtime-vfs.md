# RuntimeVfs and /spider-rt

RuntimeVfs mounts the permanent trusted Spider runtime image at `/spider-rt`.

Properties:

- kernel/Supervisor owned
- memory resident
- immutable and read-only
- not RAMFS, tmpfs, QFS, or ext4
- trusted source for `/spider-rt/sbin/spider-rs`

Supported milestone operations are lookup/read through the image parser and execute handoff to the userspace loader. Write-like operations must return read-only errors and must not silently succeed.
