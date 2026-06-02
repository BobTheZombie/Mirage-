# Mirage Kernel

Mirage is a conceptual 64-bit, Rust-based operating system kernel organised into two tightly
coupled layers:

* **Level 1 (L1) core** – handles CPU scheduling, process lifecycle management,
  message-based inter-process communication (IPC), device management, filesystem access and syscall
  dispatch.
* **Level 2 (L2) security core** – uses the `src/subkernel/` isolation domains, credentials,
  capabilities and message authorization to authenticate tasks and adjudicate message flow between
  processes.

The kernel is `#![no_std]` and now has a real x86_64 boot artifact path: Cargo builds a
freestanding ELF for a custom target, a linker script places the kernel and Limine request records,
and the Makefile packages the ELF into a BIOS/UEFI bootable ISO.

## Architecture flow

Mirage boots from the Limine request and response records declared in `src/boot.rs`. Control enters
through the `_start` entry point in `src/main.rs`, which gathers boot information and hands it to
`kernel_main`. The architecture layer is initialised with `x86_64::init_architecture` from
`src/arch/x86_64/mod.rs`, then the L1/L2 runtime is constructed with
`Kernel::<MAX_PROCESSES, MESSAGE_DEPTH>::new()`.

From there, `bootstrap_with_framebuffer` seeds framebuffer-aware boot state, `bootstrap_services`
registers the kernel service surface and the kernel settles into the `tick` loop where L1 scheduling,
process lifecycle, IPC, device, filesystem and syscall dispatch continue to pass through the L2
`src/subkernel/` checks for isolation domains, credentials, capabilities and message authorization.

## Layout

```
src/
├── arch/             # 64-bit x86 architectural scaffolding (initialisation, CPU hints)
├── bin/
│   └── qfsprogs.rs   # Host QFS tooling gated behind the `qfs-std` feature
├── boot.rs           # Limine boot protocol request/response records
├── kernel/           # L1 kernel components: scheduler, processes, IPC, devices, syscalls
│   ├── fs/           # Heap-free VFS, QFS, ext4, SSD/USB, path, inode, mount, permissions
│   └── services/     # Bootstrap/service registry support
├── libc/             # C/POSIX-shaped ABI wrappers and syscall shims
├── subkernel/        # L2 isolation domains, credentials, capabilities, message authorization
├── lib.rs            # Crate entry point that exposes the layered kernel modules
├── librust.rs        # Local runtime primitives and allocator exports
├── main.rs           # `_start` entry point wiring the boot data and layers together
└── stdlib.rs         # no-alloc stdlib-shaped import surface

targets/
└── x86_64-mirage.json # Freestanding x86_64 target: no OS, no red zone, static kernel code model

linker/
└── x86_64.ld      # Higher-half ELF layout, Limine request sections, BSS, and boot stack symbols

boot/limine/
└── limine.conf    # Limine menu entry for the Mirage kernel ELF
```

## Highlights

* **Pure Rust, no_std:** everything is written in Rust without the standard library to mirror a
  freestanding kernel environment.
* **Bootable ELF artifact:** the `x86_64-mirage` target disables the red zone and links through a
  kernel linker script at a higher-half virtual address with a 2 MiB physical load base.
* **Limine boot protocol:** the kernel embeds Limine request structures for bootloader info, stack
  size, higher-half direct map, framebuffer, memory map, RSDP and executable address data.
* **Deterministic resource management:** fixed-size tables and ring buffers are used instead of
  heap allocations, making the control flow easy to audit.
* **Bounded Linux/POSIX target:** filesystem and descriptor APIs are guided by the supported
  subset documented in `docs/linux-posix-compatibility.md`, not by an unbounded claim of complete
  Linux compatibility.
* **QFS root filesystem by default:** kernel bootstrap wires the built-in block storage device
  through QFS for the root filesystem; ext4 and SSD/USB backends remain available for explicit
  root mount requests and filesystem tooling.
* **Security-aware IPC:** every message is tagged with a security class and must be authorised by
  the L2 kernel before delivery.
* **Composable design:** the separation between the L1 core and the L2 security kernel allows
  experimentation with different scheduling policies or security models in isolation.

## Prerequisites

Install the host tools used by the reproducible image flow:

* Rust with the `rust-src` component available for the active toolchain.
* `git`, `make`, and a C toolchain for building the pinned Limine checkout.
* `xorriso` for ISO creation.
* `qemu-system-x86_64` for the emulator smoke test.

On Debian/Ubuntu-like systems, the non-Rust tools are typically installed with:

```sh
sudo apt-get install build-essential git make xorriso qemu-system-x86
```

Install the Rust source component with:

```sh
rustup component add rust-src
```

## Makefile targets and overrides

| Target | Description |
| --- | --- |
| `make rust-src` | Installs the Rust `rust-src` component for the active toolchain. |
| `make kernel` | Builds `target/x86_64-mirage/release/mirage-kernel`. |
| `make limine` | Downloads and builds the pinned Limine release into `build/limine`. |
| `make iso` | Builds the kernel and packages `build/mirage.iso`. |
| `make run-qemu` | Boots the ISO in QEMU. |
| `make clean` | Removes Cargo and build artifacts. |

The Makefile exposes a few variables for local environments and reproducible builds:

* `LIMINE_VERSION=v12.3.2` selects the pinned Limine binary release used by `make limine` and
  `make iso`; override it on the command line to test a different Limine release.
* `RUSTC_BOOTSTRAP=1` enables the nightly-only Cargo `-Z build-std` flags used for the custom
  freestanding target; override it only if your toolchain setup provides another path for those
  flags.
* `CARGO` and `RUSTUP` select the Cargo and Rustup executables used by `make kernel` and
  `make rust-src`, which is useful for wrappers or non-default toolchain locations.

## Build the kernel ELF

The Makefile invokes Cargo with `-Z build-std` because a custom freestanding target needs `core` and
`compiler_builtins` built for `targets/x86_64-mirage.json`.

```sh
make kernel
```

The ELF is written to:

```text
target/x86_64-mirage/release/mirage-kernel
```

You can also call Cargo directly:

```sh
RUSTC_BOOTSTRAP=1 cargo build --release --bin mirage-kernel \
  --target targets/x86_64-mirage.json \
  -Z build-std=core,compiler_builtins \
  -Z build-std-features=compiler-builtins-mem
```

## Build a bootable ISO

Build and package the kernel with Limine:

```sh
make iso
```

This produces:

```text
build/mirage.iso
```

The ISO flow:

1. Builds `target/x86_64-mirage/release/mirage-kernel`.
2. Clones the pinned `LIMINE_VERSION` release into `build/limine` and builds the Limine host tool.
3. Creates an ISO root containing `/boot/mirage-kernel.elf`, `/boot/limine/limine.conf`, BIOS
   Limine files, and fallback UEFI bootloaders under `/EFI/BOOT`.
4. Runs `xorriso` and `limine bios-install` to make `build/mirage.iso` BIOS/UEFI bootable.

To use a different Limine release, override the variable:

```sh
make iso LIMINE_VERSION=v12.3.2
```

## Host QFS tooling

The host-side QFS utility lives in `src/bin/qfsprogs.rs` and is gated behind the `qfs-std`
feature so it can use host/testing filesystem adapters. Build it with:

```sh
cargo build --features qfs-std --bin qfsprogs
```

The `qfs-std` feature is intended for host tools and tests only; it is not part of the default
`no_std` kernel build path used by `make kernel` or `make iso`.

## Emulator smoke test

Run the bootable ISO in QEMU with serial output connected to the terminal:

```sh
make run-qemu
```

Equivalent explicit command after `make iso`:

```sh
qemu-system-x86_64 -M q35 -m 256M -cdrom build/mirage.iso -serial stdio -display none -no-reboot -no-shutdown
```

The current kernel has no console or driver output yet, so a successful smoke test reaches the
Limine entry and then remains in the kernel tick loop without resetting or triple-faulting.

## Real hardware path

After `make iso`, write the ISO to removable media and boot it on an x86_64 machine. Replace
`/dev/sdX` with the whole target device, not a partition:

```sh
sudo dd if=build/mirage.iso of=/dev/sdX bs=4M status=progress oflag=sync
sync
```

Then select the USB device in the firmware boot menu. The image contains both legacy BIOS and UEFI
Limine paths. Secure Boot must be disabled unless you add your own signed Limine and kernel chain.

## Status

This implementation is still intentionally minimal: Limine now gets the kernel into long mode with a
memory map and framebuffer request available, but Mirage still lacks real device drivers, paging
ownership, interrupts, and a userspace loader. The boot flow is suitable as a concrete starting point
for those next pieces rather than as a general-purpose operating system.
