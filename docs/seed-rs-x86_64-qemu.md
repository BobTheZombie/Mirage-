# seed-rs x86_64 QEMU handoff

`seed-rs` is Mirage's first owned x86_64 handoff layer after Limine enters the
kernel ELF. Limine remains the boot protocol and loader: it discovers the Mirage
ELF, satisfies the Limine request sections, loads the kernel, and jumps to the
ELF entry point `_start`.

## seed-rs is the default x86_64 handoff path

`seed-rs` is now the permanent default Mirage x86_64 entry/handoff layer, not
an emergency-only or debug-only route. Limine loads the Mirage ELF and transfers
control to `_start`; `_start` installs the initial stack contract and calls
`__mirage_x86_64_seed_entry`; seed-rs then performs the single authoritative
Limine snapshot and `BootInfo::from_limine()` construction before calling
`kernel_main(boot_info)`.

Default normal flow:

```text
Limine
  -> ELF entry _start
  -> seed-rs x86_64 entry
  -> seed_rs::x86_64_handoff()
  -> limine::snapshot()
  -> BootInfo::from_limine()
  -> kernel_main(boot_info)
  -> normal Mirage boot
```

Emergency mode remains available, but it is now only an early-exit behavior
inside `kernel_main` after the same seed-rs handoff and the same `BootInfo`
construction path. Normal `make build`, `make image`, `make qemu`,
`make qemu-headless`, and `make qemu-debug` builds all use seed-rs by default.
Use `make qemu-emergency` or `QEMU_FEATURES=emergency-boot make qemu` when the
post-`BootInfo` emergency idle loop is desired.

The `[seed-rs 01]` through `[seed-rs 06]` serial markers are debug-only
breadcrumbs. Normal builds suppress them; enable the `boot-trace` feature
(or `bootdiag-verbose`, which includes it) when raw handoff breadcrumbs are
needed on COM1.

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
available through QEMU `-serial stdio` without heap allocation, formatting macros,
interrupts, framebuffer output, or the normal Mirage console. These success
breadcrumbs are emitted only when the kernel is built with `boot-trace`.

With `boot-trace`, seed markers identify the last completed handoff stage:

| Marker | Meaning |
| --- | --- |
| `[seed-rs 01] entered seed entry` | `_start` reached `__mirage_x86_64_seed_entry` and seed COM1 is initialized. |
| `[seed-rs 02] bss cleared` | The linker-provided `.bss` range was zeroed. |
| `[seed-rs 03] linker sections captured` | Kernel section ranges were captured from linker symbols. |
| `[seed-rs 04] limine snapshot captured` | Limine response pointers were snapshotted. |
| `[seed-rs 05] bootinfo constructed` | `BootInfo` was constructed from the Limine snapshot. |
| `[seed-rs 06] calling kernel_main` | Control is about to enter `kernel_main(boot_info)`. |


## BootInfo construction safety

`BootInfo::from_limine()` is reached only through `seed_rs::x86_64_handoff()`
for x86_64 Limine boots. Normal boot and emergency boot both use this same
authoritative path, so the constructor must treat every Limine response as
optional bootloader data and avoid depending on the allocator, framebuffer,
interrupts, supervisor, MTSS, or formatted logging while it builds the kernel's
first typed boot handoff.

With `boot-trace`, the constructor emits raw COM1 markers before and after each
bounded parsing step:

| Marker | Meaning |
| --- | --- |
| `[bootinfo 01] enter from_limine` | BootInfo construction started. |
| `[bootinfo 02] executable address parsed` | Optional executable load address was converted or left absent. |
| `[bootinfo 03] boot protocol parsed` | Limine base-revision status was copied. |
| `[bootinfo 04] bootloader parsed` | Optional bootloader name/version strings were bounded or left empty. |
| `[bootinfo 05] memory map parsed` | Optional memory-map response was wrapped lazily or rejected as empty/malformed. |
| `[bootinfo 06] framebuffer parsed` | Optional first framebuffer was read defensively or left absent. |
| `[bootinfo 07] serial parsed` | Serial BootInfo field was completed; seed-rs still uses raw COM1 directly. |
| `[bootinfo 08] rsdp parsed` | Optional RSDP physical address was copied or left absent. |
| `[bootinfo 09] hhdm parsed` | Optional HHDM offset was copied or left absent. |
| `[bootinfo 10] kernel image parsed` | Linker section ranges and optional load range were assembled. |
| `[bootinfo 11] modules parsed` | Optional module response was wrapped lazily or replaced by an empty module list. |
| `[bootinfo 12] BootInfo return` | A complete `BootInfo` value is about to return to seed-rs. |

Additional helper markers identify malformed Limine data without using
`kprintln!` or formatting. `BootString::from_cstr()` reports null-pointer checks
and the bounded scan window; it scans at most 256 bytes and stores the raw byte
slice without allocating or assuming UTF-8. A null bootloader, module path, or
module command-line pointer becomes an empty `BootString`.

Memory-map construction is lazy. A missing memory-map response, a zero entry
count, a null entry-vector pointer, or an unaligned entry-vector pointer produces
`None` instead of dereferencing the map during construction. Individual entries
are read only through `MemoryMap::entry(index)`, which bounds-checks the index,
checks the entry-vector pointer, checks the selected entry pointer, and returns
`None` for malformed or out-of-range entries.

Boot module construction is also lazy. A missing module response, zero module
count, null module-vector pointer, or unaligned module-vector pointer produces
`BootModules::empty()`. Module file records and their path/cmdline strings are
parsed only when `BootModules::module(index)` is called, and invalid indexes or
null/unaligned module pointers return `None`.

Framebuffer handling reads only `framebuffers[0]`. A missing framebuffer
response, zero framebuffer count, null framebuffer-vector pointer, null first
framebuffer pointer, or unaligned pointer returns `None`. The wrapper preserves
the 64-bit framebuffer address, copies Limine's pitch directly, and deliberately
does not inspect mode lists or EDID data during BootInfo construction.

When a `boot-trace` build hangs or stops before `[seed-rs 05] bootinfo constructed`,
compare the last `[bootinfo ..]` or helper marker on serial with the tables above. A stop
between a helper's pointer/count marker and its return marker usually indicates a
malformed Limine response in that helper. Valid malformed optional responses
should now degrade to `None`, an empty `BootString`, or `BootModules::empty()` so
normal and emergency boot can continue to `kernel_main()`.

## QEMU emergency mode

Emergency mode no longer selects a separate seed-rs route. The same default
seed-rs handoff runs first, constructs `BootInfo`, and calls `kernel_main`. The
`emergency-boot` feature then makes `kernel_main` print the emergency marker and
enter the x86_64 halt loop before normal architecture initialization or
heap-dependent work. The legacy `seed-rs-qemu-emergency` feature is retained as
an alias for `emergency-boot`.

Expected serial output after Limine output in a normal non-trace emergency build:

```text
Mirage emergency boot reached idle loop
```

A `boot-trace` emergency build also includes the seed-rs and BootInfo breadcrumbs
before the emergency line.

## Run

```sh
make qemu
```

The normal QEMU target builds the default Mirage image through seed-rs. It no
longer requires `QEMU_FEATURES=emergency-boot`. The image build stages the
kernel, runs `tools/verify-seed-rs-elf.sh`, and launches QEMU using the generated
mirageconfig QEMU arguments.

Compatibility seed targets such as `make qemu-seed` use the same normal seed-rs
entry path; they are no longer the only way to exercise seed-rs.

## Debug

```sh
make qemu-seed-debug
```

The debug targets use the same image and validation path as normal QEMU, but add
QEMU's GDB stub flags:

```text
-S -s
```

Attach with GDB:

```gdb
target remote :1234
```

## Inspect QEMU failures

The QEMU targets leave the diagnostic log at:

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
kernel and staged kernel copy. It also verifies that `_start` dispatches to
`__mirage_x86_64_seed_entry` and warns when the legacy
`__mirage_x86_64_bootstrap` compatibility symbol is still present but unused. It
fails if required symbols are missing, the ELF entry point does not equal
`_start`, the seed-rs dispatch cannot be proven, any Limine request section is
missing, or the staged kernel hash differs from the built kernel hash.
