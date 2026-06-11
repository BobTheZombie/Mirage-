# Emergency Boot Debugging

This guide documents the narrow emergency boot path used to verify the Limine to
Mirage handoff before the normal kernel, supervisor, memory, QFS, or service
policy paths are involved. The emergency path is intentionally small: it prints
stable COM1 markers, proves that `kernel_main` was entered, then halts in the
architecture panic loop after a final idle-loop diagnostic.

## Run the emergency boot target

From the repository root, run:

```sh
make qemu-emergency
```

The target builds the QEMU ISO with only the `emergency-boot` feature enabled,
starts QEMU with serial output connected to the terminal, and enables QEMU
interrupt/reset tracing in `build/qemu.log`.

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
[MIRAGE BOOT 01]
[MIRAGE BOOT 02]
[MIRAGE BOOT 03]
[MIRAGE BOOT 04]
[MIRAGE BOOT 05]
[MIRAGE BOOT 06]
[MIRAGE BOOT 07]
Mirage emergency boot reached idle loop
```

Limine may print its own bootloader diagnostics before the Mirage markers. Treat
`[MIRAGE BOOT 01]` as the first line emitted by Mirage itself.

## Interpreting the last printed marker

When the VM stops, resets, triple-faults, or appears to hang, find the last
`[MIRAGE BOOT NN]` line printed on the serial console. The next stage after that
line is the most likely failing boundary. For example:

- Last line is `[MIRAGE BOOT 01]`: Limine reached Mirage `_start`, but execution
  failed before or while entering the Rust bootstrap routine.
- Last line is `[MIRAGE BOOT 02]`: Mirage entered the bootstrap routine, but
  failed while clearing `.bss` or before the post-`.bss` marker could print.
- Last line is `[MIRAGE BOOT 03]`: `.bss` was cleared, but the kernel section
  snapshot or Limine request snapshot did not complete.
- Last line is `[MIRAGE BOOT 04]`: Limine request state was snapshotted, but the
  typed `BootInfo` handoff was not constructed successfully.
- Last line is `[MIRAGE BOOT 05]`: `BootInfo` was constructed, but the final
  pre-`kernel_main` handoff marker or call boundary was not reached.
- Last line is `[MIRAGE BOOT 06]`: the bootstrap code reached the call to
  `kernel_main`, but Rust kernel entry did not emit its entry marker.
- Last line is `[MIRAGE BOOT 07]`: `kernel_main` was entered. In the emergency
  target, the next expected line is `Mirage emergency boot reached idle loop`.
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
| `[MIRAGE BOOT 01]` | `_start` | Limine transferred control to the Mirage ELF entry point. The assembly stub installed the bootstrap stack, aligned it, cleared `rbp`, and proved the raw COM1 writer can emit a line before calling `__mirage_x86_64_bootstrap`. |
| `[MIRAGE BOOT 02]` | Bootstrap entry | `__mirage_x86_64_bootstrap` was entered from `_start`. The next operation is clearing the kernel `.bss` range. |
| `[MIRAGE BOOT 03]` | `.bss` clear | The linker-provided `.bss` range was zeroed. The next operations read linker section bounds and snapshot Limine request state. |
| `[MIRAGE BOOT 04]` | Limine snapshot | Mirage captured the raw Limine handoff state, including base-revision, memory-map, framebuffer, module, and RSDP request data where present. The next operation converts that snapshot into typed boot data. |
| `[MIRAGE BOOT 05]` | `BootInfo` construction | The typed `BootInfo` structure was built from the Limine snapshot and kernel section metadata. |
| `[MIRAGE BOOT 06]` | `kernel_main` handoff boundary | Bootstrap is about to call `kernel_main(boot_info)`. If this marker prints but marker 07 does not, suspect the Rust ABI handoff, stack state, calling convention, or `kernel_main` prologue. |
| `[MIRAGE BOOT 07]` | `kernel_main` entry | The Rust kernel entry point was reached. In `make qemu-emergency`, this should be followed by `Mirage emergency boot reached idle loop`. |
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
