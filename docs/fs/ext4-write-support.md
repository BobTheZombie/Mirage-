# ext4 write support policy

Mirage does not enable unsafe ext4 writes by default.

## Current status

The ext4 backend has read support and explicit write refusal. `pwrite`, create, mkdir, unlink, rename, chmod/chown, and truncate return read-only errors through the VFS until all metadata safety requirements are complete.

## Required before read-write mode

- Block bitmap allocation and free-block counter updates.
- Inode bitmap allocation, inode initialization, and free-inode counter updates.
- Existing-block read-modify-write and full-block writes.
- File growth through extent allocation and extent-tree mutation.
- Directory entry insertion/removal and link count updates.
- Superblock and group descriptor metadata updates.
- Dirty data/metadata cache with bounded memory and flush.
- Checksum updates for checksum-enabled metadata.
- JBD2 replay and metadata transaction commit, or refusal of read-write mounts on journaled volumes.

## Journal policy

Journaled ext4 may be mounted read-only for reads. Read-write mounting a journaled ext4 volume must fail with `JournalRequired` until JBD2 replay/commit is implemented. Non-journaled images created for Mirage experiments are the only acceptable first target for future read-write enablement.
