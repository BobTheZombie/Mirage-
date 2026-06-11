# seed-rs x86_64 QEMU handoff

`seed-rs` is Mirage's first owned x86_64 handoff layer after Limine enters the
kernel ELF. Limine remains the boot protocol and loader: it discovers the Mirage
ELF, satisfies the Limine request sections, loads the kernel, and jumps to the
ELF entry point `_start`.

## What seed-rs is

`seed-rs` is a minimal Rust `no_std` handoff boundary for the current hardware
bring-up path:

```text
Limine
  -> ELF entry _start
  -> x86_64 assembly stub
  -> __mirage_x86_64_seed_entry
  -> seed_rs::x86_64_handoff()
  -> kernel_main(boot_info)
```

The layer is intentionally x86_64, QEMU, and Limine focused. It performs the
first Mirage-owned sequencing steps before normal kernel logging, framebuffer,
allocator-backed code, supervisor policy, MTSS, drivers, userspace, or filesystem
work can be trusted.

## What seed-rs is not

`seed-rs` is not a bootloader replacement, not a generic multi-architecture boot
framework, and not a place for policy. It does not implement a memory allocator,
framebuffer console, supervisor bootstrap, MTSS, driver model, userspace launch,
or filesystem initialization.

## Raw COM1 diagnostics

The seed path uses raw x86_64 port I/O against COM1 (`0x3f8`) so progress is
visible through QEMU `-serial stdio` without heap allocation, formatting macros,
interrupts, framebuffer output, or the normal Mirage console.

Seed markers identify the last completed handoff stage:

| Marker | Meaning |
| --- | --- |
| `[seed-rs 01] entered seed entry` | `_start` reached `__mirage_x86_64_seed_entry` and seed COM1 is initialized. |
| `[seed-rs 02] bss cleared` | The linker-provided `.bss` range was zeroed. |
| `[seed-rs 03] linker sections captured` | Kernel section ranges were captured from linker symbols. |
| `[seed-rs 04] limine snapshot captured` | Limine response pointers were snapshotted. |
| `[seed-rs 05] bootinfo constructed` | `BootInfo` was constructed from the Limine snapshot. |
| `[seed-rs 06] calling kernel_main` | Control is about to enter `kernel_main(boot_info)`. |

## QEMU emergency mode

The `seed-rs-qemu-emergency` Cargo feature proves the handoff without entering
normal architecture initialization or heap-dependent work. In this mode,
`kernel_main` immediately prints through seed-rs COM1 and enters the x86_64 halt
loop.

Expected serial output after Limine output:

```text
[seed-rs 01] entered seed entry
[seed-rs 02] bss cleared
[seed-rs 03] linker sections captured
[seed-rs 04] limine snapshot captured
[seed-rs 05] bootinfo constructed
[seed-rs 06] calling kernel_main
Mirage seed-rs QEMU emergency boot reached idle loop
```

## Run

```sh
make qemu-seed
```

The target builds the kernel with:

```text
--no-default-features --features seed-rs-qemu-emergency
```

It then rebuilds the ISO staging tree, copies the built kernel into the image,
runs `tools/verify-seed-rs-elf.sh`, and launches QEMU with serial output on the
terminal. The target intentionally does **not** pass `-S`, so QEMU should not
start paused.

## Debug

```sh
make qemu-seed-debug
```

The debug target uses the same image and validation path as `make qemu-seed`, but
adds QEMU's GDB stub flags:

```text
-S -s
```

Attach with GDB:

```gdb
target remote :1234
```

## Inspect QEMU failures

Both seed targets leave the QEMU diagnostic log at:

```text
build/qemu.log
```

If QEMU exits, resets, or appears to hang before the expected seed output, inspect
that file for symptoms such as:

- triple fault
- page fault
- CPU reset
- bad entry address
- invalid instruction

The last seed marker printed to serial narrows the failing handoff stage.

## Verify without launching QEMU

After building and staging an image, run:

```sh
tools/verify-seed-rs-elf.sh
```

The verifier prints the ELF entry point, symbol addresses for `_start`,
`__mirage_x86_64_seed_entry`, optional `__mirage_x86_64_bootstrap`, and
`kernel_main`, Limine request-section presence, and SHA256 hashes for the built
kernel and staged kernel copy. It fails if required symbols are missing, the ELF
entry point does not equal `_start`, any Limine request section is missing, or the
staged kernel hash differs from the built kernel hash.
