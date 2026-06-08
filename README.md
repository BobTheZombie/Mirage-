# Mirage Kernel

Mirage is a conceptual 64-bit, Rust-based GNU/Mirage operating system kernel organised
around a mechanism/policy split:

* **Mechanism-only kernel layer** – provides CPU scheduling primitives, process lifecycle
  mechanics, message-based inter-process communication (IPC), filesystem mechanisms and syscall
  entry points without treating POSIX or Linux conventions as internal design constraints.
* **Supervisor and security broker layers** – combine `src/supervisor/` policy/recovery
  orchestration with the `src/subkernel/` isolation domains, credentials, capabilities and message
  authorization used to authenticate tasks, broker security decisions and recover supervised
  services.

The kernel is `#![no_std]` and now has a real x86_64 boot artifact path: Cargo builds a
freestanding ELF for a custom target, a linker script places the kernel and Limine request records,
and the Makefile packages the ELF with a signed boot module set into a BIOS/UEFI bootable ISO.

## Architecture flow

Mirage boots from the Limine request and response records declared in `src/boot.rs`. The
handoff sequence is: Limine populates those requests, transfers control to the `_start` assembly
stub in `src/arch/x86_64/boot.rs`, `_start` calls `__mirage_x86_64_bootstrap` in the same file,
the bootstrap clears `.bss`, snapshots the Limine state into `BootInfo`, and then hands that
snapshot to `src/main.rs::kernel_main`. Serial output begins with the stable
`Mirage kernel booting` marker once `kernel_main` starts.

`kernel_main` checks the Limine base revision, initialises the architecture with
`x86_64::init_architecture` from `src/arch/x86_64/mod.rs`, constructs the mechanism-only kernel with
`Kernel::<MAX_PROCESSES, MESSAGE_DEPTH>::new()`, and applies boot state through
`kernel.bootstrap_with_boot_info(&boot_info)`. The default non-`full-boot` path then runs the
minimal supervisor bootstrap, explicitly skips QFS root mounting and userspace init, and admits the
compiled-in mock manifest for the `echo-service` IPC smoke path. Building with `full-boot` instead
attempts the fuller boot-source root mount, service manifest startup, and userspace init paths,
which may still report skipped or stubbed work while those milestones are incomplete.

In both paths, the supervisor remains the policy/recovery/security broker for services, including
supervised driver services as the preferred driver model, while the kernel settles into the `tick`
loop after the stable `Mirage reached idle loop` marker. Scheduling, process lifecycle, IPC,
filesystem mechanisms and external ABI dispatch continue to pass through `src/subkernel/` checks
for isolation domains, credentials, capabilities and message authorization.

## Layout

```
src/
├── arch/             # 64-bit x86 bootstrap, typed boot handoff, and CPU scaffolding
├── bin/
│   └── qfsprogs.rs   # Host QFS tooling gated behind the `qfs-std` feature
├── boot.rs           # Limine boot protocol request/response records
├── kernel/           # Mechanism-only scheduler, processes, IPC, devices, syscalls
│   ├── fs/           # Heap-free VFS, native indexed-object QFS, ext4, path, inode, mount
│   └── services/     # Bootstrap/service registry support for supervised services
├── libc/             # C/POSIX-shaped external ABI wrappers and syscall shims
├── subkernel/        # Isolation domains, credentials, capabilities, message authorization
├── supervisor/       # Policy, recovery, security brokerage and signed service manifests
├── lib.rs            # Crate entry point that exposes the layered kernel modules
├── librust.rs        # Local runtime primitives and allocator exports
├── main.rs           # `kernel_main` entry that wires boot data into kernel/supervisor layers
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
* **External POSIX/GNU compatibility surface:** filesystem and descriptor APIs expose a bounded
  external ABI guided by `docs/linux-posix-compatibility.md`; Mirage internals remain GNU/Mirage
  mechanisms rather than POSIX or Linux design assumptions.
* **Native QFS object filesystem:** QFS is the native indexed object filesystem and default root
  filesystem; ext4 and block backends remain available for explicit compatibility mounts and
  filesystem tooling.
* **Supervised driver services:** device-facing daemons are launched from the signed boot module set
  and supervised as recoverable services, which is the preferred GNU/Mirage driver model.
* **Security-aware IPC:** every message is tagged with a security class and must be authorised by
  the brokered security layer before delivery.
* **Composable design:** the separation between the mechanism-only kernel, supervisor policy and
  security broker layers allows experimentation with different scheduling policies or security
  models in isolation.

## Prerequisites

Install the host tools used by the reproducible image flow:

* Rust with the `rust-src` component available for the active toolchain.
* `git`, `make`, and a C toolchain for building the pinned Limine checkout.
* `xorriso` for ISO creation.
* `qemu-system-x86_64` for the emulator smoke test.
* `readelf` or `llvm-readelf` for the x86_64 boot artifact smoke test.

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
| `make smoke-x86_64-boot` | Builds and validates the x86_64 kernel ELF boot artifact without launching an emulator. |
| `make clean` | Removes Cargo and build artifacts. |

The Makefile exposes a few variables for local environments and reproducible builds:

* `LIMINE_VERSION=v12.3.2` selects the pinned Limine binary release used by `make limine` and
  `make iso`; override it on the command line to test a different Limine release.
* `RUSTC_BOOTSTRAP=1` enables the nightly-only Cargo `-Z build-std` flags used for the custom
  freestanding target; override it only if your toolchain setup provides another path for those
  flags.
* `CARGO`, `RUSTC`, and `RUSTUP` select the Cargo, Rust compiler, and Rustup executables
  used by `make kernel` and `make rust-src`, which is useful for wrappers or non-default
  toolchain locations. For example:

  ```sh
  make kernel CARGO="$(rustup which cargo)" RUSTC="$(rustup which rustc)"
  ```

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
   Limine files, signed boot module metadata, and fallback UEFI bootloaders under `/EFI/BOOT`.
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

The QEMU smoke path now uses COM1 serial diagnostics, so a successful boot should visibly print
markers such as `Mirage kernel booting` and `Mirage reached idle loop` before remaining in the
idle loop without resetting or triple-faulting. For CI and local automation, use
`scripts/qemu-smoke.sh`; it builds the ISO, captures the serial log, and checks for the expected
boot markers.

For a non-emulator x86_64 boot artifact baseline, use:

```sh
make smoke-x86_64-boot
```

This runs `scripts/x86_64-boot-smoke.sh`, which builds `make kernel` by default and then checks
that the linked ELF is an ELF64 x86_64 image whose entry address resolves to `_start`. It also
verifies the required bootstrap/linker symbols (`_start`, `__mirage_x86_64_bootstrap`,
`__limine_requests_start`, `__limine_requests_end`, `__stack_top`, `__bss_start`, and
`__bss_end`) and confirms that `.requests`, `.requests_start_marker`, and
`.requests_end_marker` survived section garbage collection. To inspect an already-built or
external artifact, pass `KERNEL_ELF=/path/to/mirage-kernel` to the script.

## Real hardware path

After `make iso`, write the ISO to removable media and boot it on an x86_64 machine. Replace
`/dev/sdX` with the whole target device, not a partition:

```sh
sudo dd if=build/mirage.iso of=/dev/sdX bs=4M status=progress oflag=sync
sync
```

Then select the USB device in the firmware boot menu. The image contains both legacy BIOS and UEFI
Limine paths. Secure Boot must be disabled unless you add your own signed Limine, kernel, and
boot-module chain.

## Status

This implementation is still intentionally minimal: Limine now gets the kernel into long mode with a
memory map and framebuffer request available, but Mirage still lacks real device drivers, paging
ownership, interrupts, and a userspace loader. The boot flow is suitable as a concrete starting point
for those next pieces rather than as a general-purpose operating system.
