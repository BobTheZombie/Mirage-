# ext4 on Mirage

Mirage ext4 is a no-std, Rust-only filesystem backend that sits above Mirage block devices. It is not a Linux compatibility shortcut and does not bypass the block layer.

## Current support

- Block-backed mount through `BlockStorageDevice`.
- Superblock parsing and magic validation (`0xEF53`).
- Block-size, inode-size, block-group, inode-table, feature, UUID, and volume-name parsing.
- 32-bit and 64-bit block group descriptor parsing.
- Inode loading by inode number.
- Extent leaf and indexed extent-tree reads.
- Legacy direct and single-indirect block-map reads.
- Linear directory scans with ext4 file-type fields and record-length validation.
- Arbitrary offset/length file reads, cross-block reads, EOF handling, and sparse holes.
- Conservative feature policy with read-only default.

## Feature policy

Mount fails for unsupported incompatible or read-only-compatible feature bits. The initial supported read policy includes extents, filetype, 64-bit descriptors, flex_bg descriptor placement, sparse_super, large_file, huge_file, metadata_csum as read-only, dir_index as linear-read optional, and journal-present volumes as read-only.

Writes are only allowed by future explicit read-write mount policy when the volume is non-journaled and all required metadata update/checksum paths are implemented. Journaled ext4 write attempts are refused until JBD2 support exists.

## Known limitations

- No JBD2 replay or commit support yet.
- No safe ext4 metadata write implementation yet.
- No HTree lookup; indexed directories are scanned linearly where possible.
- No double/triple indirect legacy reads.
- No shared block cache yet.
