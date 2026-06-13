# qfs-mkimage

`qfs-mkimage` is currently provided by the hosted Rust binary `qfsprogs`:

```sh
cargo run --features qfs-std --bin qfsprogs -- mkfs build/qfs.img
cargo run --features qfs-std --bin qfsprogs -- fsck build/qfs.img
cargo run --features qfs-std --bin qfsprogs -- stat build/qfs.img /
```

The Makefile target `make qfs-image` creates and validates `build/qfs.img`.
The AHCI/NVMe QEMU targets create separate raw QFS images for each controller path.
