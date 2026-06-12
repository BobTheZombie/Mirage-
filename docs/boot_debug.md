# Emergency Boot Debugging

This guide documents the narrow emergency boot behavior used to verify the
Limine -> `_start` -> seed-rs -> `BootInfo` -> `kernel_main` handoff before the
normal kernel, supervisor, memory, QFS, or service policy paths are involved.
seed-rs is the default x86_64 handoff path for both normal and emergency boots;
emergency mode only changes `kernel_main` so it exits early to the architecture
panic loop after a final idle-loop diagnostic.

## Run the emergency boot target

From the repository root, run:

```sh
make qemu-emergency
```

The target builds the normal seed-rs QEMU ISO with the `emergency-boot` feature
enabled, starts QEMU with serial output connected to the terminal, and enables
QEMU interrupt/reset tracing in `build/qemu.log`.

Equivalent pieces supplied by the Makefile target are:

```text
QEMU_FEATURES=emergency-boot
MIRAGE_QEMU_SERIAL_ARGS="-serial stdio"
MIRAGE_QEMU_DEBUG_ARGS="-d int,cpu_reset -D build/qemu.log"
```

## Expected serial output

After Limine has loaded the kernel and transferred control to Mirage, the serial
stream should contain boot markers in increasing order. A successful emergency
boot should include output like:

```text
[seed-rs 01] entered seed entry
[seed-rs 02] bss cleared
[seed-rs 03] linker sections captured
[seed-rs 04] limine snapshot captured
[seed-rs 05] bootinfo constructed
[seed-rs 06] calling kernel_main
Mirage emergency boot reached idle loop
```

Limine may print its own bootloader diagnostics before the Mirage markers. Treat
`[seed-rs 01] entered seed entry` as the first line emitted by Mirage itself.

## Interpreting the last printed marker

When the VM stops, resets, triple-faults, or appears to hang, find the last
`[seed-rs NN]` line printed on the serial console. The next stage after that
line is the most likely failing boundary. For example:

- Last line is `[seed-rs 01]`: Limine reached Mirage `_start` and `_start`
  called `__mirage_x86_64_seed_entry`, but execution failed before or while
  clearing `.bss`.
- Last line is `[seed-rs 02]`: seed-rs cleared `.bss`, but failed while reading
  linker section boundaries or before the next marker could print.
- Last line is `[seed-rs 03]`: `.bss` was cleared, but the kernel section
  snapshot or Limine request snapshot did not complete.
- Last line is `[seed-rs 04]`: Limine request state was snapshotted, but the
  typed `BootInfo` handoff was not constructed successfully.
- Last line is `[seed-rs 05]`: `BootInfo` was constructed, but the final
  pre-`kernel_main` handoff marker or call boundary was not reached.
- Last line is `[seed-rs 06]`: seed-rs reached the call to `kernel_main`. In the
  emergency target, the next expected line is
  `Mirage emergency boot reached idle loop`.
- Last line is `[MIRAGE BOOT 08]` or `[MIRAGE BOOT 09]`: these are normal-boot
  architecture-initialization boundaries. They are not expected from
  `make qemu-emergency`, because the emergency target deliberately halts before
  the full architecture setup path.

Always interpret the marker as "the previous boundary completed" rather than
"the marker itself failed." The fault is usually in the stage immediately after
the last printed marker.

## Marker map

| Marker | Boundary completed | Meaning |
| --- | --- | --- |
| `[seed-rs 01] entered seed entry` | seed-rs entry | `_start` transferred control to `__mirage_x86_64_seed_entry`; seed-rs initialized raw COM1 diagnostics. |
| `[seed-rs 02] bss cleared` | `.bss` clear | The linker-provided `.bss` range was zeroed. |
| `[seed-rs 03] linker sections captured` | Linker metadata | Kernel section ranges were captured from linker symbols. |
| `[seed-rs 04] limine snapshot captured` | Limine snapshot | Mirage captured raw Limine response pointers. |
| `[seed-rs 05] bootinfo constructed` | `BootInfo` construction | The typed `BootInfo` structure was built from the Limine snapshot and kernel section metadata. |
| `[seed-rs 06] calling kernel_main` | `kernel_main` handoff boundary | seed-rs is about to call `kernel_main(boot_info)`. In `make qemu-emergency`, this should be followed by `Mirage emergency boot reached idle loop`. |
| `[MIRAGE BOOT 08]` | Architecture init start boundary | Normal, non-emergency boots print this immediately before calling `init_architecture(&boot_info)`. It marks the boundary from top-level kernel entry into x86_64 architecture setup. |
| `[MIRAGE BOOT 09]` | Architecture init complete boundary | Normal, non-emergency boots print this after `init_architecture(&boot_info)` returns. It means descriptor tables, early serial, memory layout, framebuffer, and interrupt-controller setup completed for the enabled feature set. |

## Inspect QEMU interrupt and reset logs

The emergency target writes QEMU diagnostic logs to:

```sh
less build/qemu.log
```

Useful searches inside `less` include:

```text
/CPU Reset
```

Use this to identify where QEMU reset the virtual CPU. A reset after only one or
two Mirage markers usually means the fault happened very early in the handoff or
bootstrap path.

```text
/check_exception
```

Use this to find exceptions recorded by QEMU's `-d int` trace. Consecutive
exception lines often show the escalation path from the original fault to a
double fault and then a reset.

```text
/Triple fault
```

Use this to search for triple-fault diagnostics. A triple fault means the CPU
could not dispatch the exception path, usually because the IDT, stack, page
mapping, or exception handler state was invalid for the fault that occurred.

```text
/Servicing hardware INT
```

Use this to inspect hardware interrupt delivery. Unexpected interrupts before
Mirage has installed the normal architecture path can point to interrupt masking,
PIC/APIC, or firmware handoff assumptions.

Additional helpful searches are:

```text
/v=
```

for exception-vector summaries in QEMU interrupt traces, and:

```text
/EIP=
```

or:

```text
/RIP=
```

for the guest instruction pointer around reset or exception records.

## Verify the ELF handoff shape

Before chasing an emulator-only problem, verify that the linked kernel ELF still
matches the Limine handoff contract:

```sh
tools/verify-kernel-elf.sh
```

The script checks that the built kernel is an ELF64 x86_64 image, that the entry
address resolves to `_start`, and that required bootstrap/linker symbols and
Limine request sections survived linking. If this check fails, fix the ELF or
linker shape before debugging QEMU runtime behavior.
