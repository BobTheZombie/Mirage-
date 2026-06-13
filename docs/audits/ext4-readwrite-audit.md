# Mirage ext4 read/write storage audit

## Scope

Audited the kernel storage path needed for:

```text
AHCI/NVMe/M.2 hardware -> BlockStorageDevice/BlockDevice -> VFS -> ext4 -> root/userspace files
```

This audit is intentionally conservative. Mirage must not claim ext4 is `Online` or writable unless a real block-backed volume is mounted and the safety policy permits the operation.

## Working

- `crates/mirage-block` provides a no-std `BlockDevice` abstraction, stable `BlockDeviceId`, request ranges, device state, fixed-capacity registry, read/write/flush methods, and mock-device tests.
- `crates/mirage-storage` wraps `mirage-block` devices behind supervisor-style capabilities and exposes read-only/read-write grants, block read/write, flush, hotplug events, and placeholder MBR/GPT discovery.
- `crates/mirage-ahci` and `crates/mirage-nvme` expose real driver-facing storage identities and block-device metadata, but the kernel root path still depends on registered `BlockStorageDevice` drivers becoming visible through the device manager.
- `src/kernel/device.rs` has a kernel `BlockStorageDevice` trait used by QFS and ext4, including sector-aligned read/write/flush/discard hooks.
- QFS has a real block-backed mount path in `QfsFileSystem::new_on_block_device` plus superblock/journal/inode refresh from a `BlockStorageDevice`.
- VFS already exposes POSIX-like lookup/open/pread/pwrite/readdir/stat hooks and a fixed mount table.
- Kernel root selection can mount QFS and ext4 from boot modules or enumerated block-storage devices; discovered ext4 is now tried before QFS for auto block roots.
- The ext4 module parses superblocks, block group descriptors, inodes, extent records, directory entries, bitmaps, checksums, and journal record headers.
- ext4 read path now validates the root inode during mount, applies an explicit feature policy, supports extent leaf and indexed extent trees, supports legacy direct and single-indirect block maps, scans linear directories, handles sparse holes as zeroes, and performs arbitrary-offset reads.
- QEMU has QFS image targets and now has ext4 image/QEMU AHCI/NVMe targets.

## Missing

- There is no full kernel command-line parser wired to `root=ext4:nvme0n1`, `root=ext4:sata0`, `rootflags=rw`, or `root=auto`; Make targets pass these strings for the next boot-policy milestone.
- The root mount path does not yet bind a requested root token to a specific block device name; it enumerates registered block devices.
- The userspace loader still has a stub `load_elf_from_file` helper, while kernel process exec uses the root VFS directly.
- There is no shared VFS block cache; ext4 currently uses bounded stack blocks for metadata/data reads.
- ext4 has no JBD2 transaction replay/commit implementation.
- ext4 write allocation, inode allocation, directory mutation, extent-tree growth/splitting, counter updates, checksum updates, and safe metadata writeback are not complete.
- Double and triple indirect legacy block maps are not implemented.
- HTree lookup is not implemented; indexed directories are read by linear scan when the directory data permits that.

## Unsafe / intentionally blocked

- ext4 mounts are read-only by default.
- Journaled ext4 volumes are readable but read-write mounting is refused with `JournalRequired`/read-only policy until JBD2 exists.
- `metadata_csum` volumes are readable, but writes remain blocked because metadata checksum updates are not implemented for all mutated structures.
- Unsupported incompatible or read-only-compatible feature bits fail mount instead of being ignored.
- ext4 write syscalls return `ReadOnly`/`JournalRequired`; no accidental metadata writes are issued.
- No infinite waits were added; all ext4 operations are bounded by caller-provided buffers and on-disk extents/directory sizes.

## Blockers for ext4 read

- Real hardware read depends on AHCI/NVMe devices being registered as `BlockStorageDevice` instances in the kernel device manager.
- Volumes using unsupported incompatible/ro-compatible features correctly fail instead of mounting.
- Direct metadata checksum verification is incomplete, so `metadata_csum` is treated conservatively for writes.

## Blockers for ext4 write

- JBD2 journal replay/commit is absent.
- Block/inode bitmap allocation and counter updates are not complete.
- Extent-tree mutation, splitting, and checksum updates are not complete.
- Directory insertion/removal and link-count updates are not complete.
- No dirty block cache/writeback layer exists.

## Blockers for rootfs-on-ext4

- Root auto selection is improved but still not a full `root=`/`rootflags=` implementation.
- Userspace loader status must be tied to successful VFS reads and ELF mapping, not merely file discovery.
- QEMU smoke requires host ext4 tooling and a boot path where AHCI/NVMe hardware drivers register their block devices.
