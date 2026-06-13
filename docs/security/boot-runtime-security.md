# Boot Runtime Security

Boot Runtime is read-only, immutable, and kernel-owned. Userspace and Spider-rs may read it but may not write it.

Implemented now:

- manifest range validation
- per-file hash verification using CRC32
- read-only RAMFS enforcement
- explicit failure when Spider-rs-required Boot Runtime is missing

Not implemented yet:

- cryptographic signature verification
- measured boot
- TPM policy
- revocation after root handoff
