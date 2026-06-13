# Boot Runtime RAMFS

Mirage Boot Runtime is a kernel-managed immutable RAMFS mounted at `/bootrt`. It is separate from the normal root filesystem and exists so Spider-rs PID 1 can be found even when AHCI, NVMe, QFS, ext4, or root mount fails.

Required files:

- `/bootrt/sbin/spider-rs`
- `/bootrt/etc/spider/default.target`
- `/bootrt/etc/spider/basic.target`
- `/bootrt/manifest`

The current image format is a simple binary manifest and payload archive. The kernel validates magic, file ranges, and per-file CRC32 hashes. Signature verification, measured boot, and TPM integration are future work and must not be claimed as complete.
