# Zinnia Reference Audit for Mirage Boot and Userspace Bring-up

## Zinnia commit inspected

`9c964e184874eee4ef05a7bb10200b06915e0dad`

## Zinnia license summary

Zinnia's top-level `LICENSE` is GPL-2.0. Mirage did not copy Zinnia code in this change. Zinnia was used only as an external OS reference for architecture and sequencing patterns; all Mirage changes here are independently reimplemented Mirage code.

## Zinnia files inspected

* `LICENSE`
* `README.md`
* `kernel/src/boot/limine.rs`
* `kernel/src/boot/mod.rs`
* `kernel/src/lib.rs`
* `kernel/src/module.rs`
* `kernel/src/sched/mod.rs`
* `kernel/src/process/mod.rs`
* `kernel/src/process/task.rs`
* `kernel/src/syscall/mod.rs`
* `kernel/src/syscall/process.rs`
* `kernel/src/syscall/vfs.rs`
* `kernel/src/vfs/fs/initramfs.rs`
* `kernel/src/vfs/mod.rs`
* `kernel/src/memory/user.rs`
* `kernel/src/arch/x86_64/sched.rs`
* `kernel/src/arch/x86_64/irq.rs`
* `kernel/src/device/mod.rs`
* `kernel/src/device/pci/mod.rs`
* `kernel/src/device/usb/mod.rs`
* `drivers/usb/xhci/src/lib.rs`
* `drivers/usb/xhci/src/spec.rs`
* `kernel/src/log.rs`
* `kernel/src/panic.rs`

## Relevant Zinnia architecture summary

Zinnia uses Limine requests to normalize bootloader state into an internal `BootInfo`, including memory map, framebuffer, paging mode, command line, and boot files. It then treats bootloader files as either initramfs archives loaded into VFS or RAM disks exposed as block devices. Scheduler and process code use explicit task states, runnable queues, an idle task, reschedule flags, and reaping structures. Syscall handling is table/dispatcher oriented and converts architecture contexts into typed syscall results. Device bring-up uses registration/probe patterns and module entry metadata. Logging uses named sinks and keeps panic diagnostics distinct from ordinary log paths.

## Mirage equivalents

* Limine normalization: `src/arch/x86_64/boot.rs`, `src/kernel/boot_runtime.rs`.
* Runtime image: `build/spider-rt.img`, `src/kernel/boot_runtime.rs`, `tools/validate-boot-runtime.sh`.
* PID1 loader: `src/kernel/userspace/elf_loader.rs`, `src/kernel/userspace/abi.rs`, `src/kernel/mod.rs`.
* MTSS task admission: `crates/mirage-mtss`, `src/kernel/mod.rs`.
* Device registry: `src/kernel/device.rs`, `crates/mirage-platform`.
* Boot UI/log split: `src/kernel/boot_screen.rs`, `src/arch/x86_64/early_console.rs`, `src/arch/x86_64/framebuffer_console.rs`.

## What Mirage can learn

1. Normalize bootloader modules once and validate their role before use.
2. Keep early runtime payload validation strict and fail closed when mandatory files are absent.
3. Treat userspace entry as a chain of proofs: ELF header, PT_LOAD bounds, non-overlap, executable mapped entry, writable stack, address-space/CR3 proof, then runnable admission.
4. Keep scheduler state transitions explicit rather than conflating created, runnable, and running.
5. Keep verbose diagnostics on serial/log sinks while framebuffer milestone UI remains concise.

## What Mirage should not copy

* Zinnia's GPL-2.0 implementation code.
* Zinnia's monolithic initramfs-as-root policy; Mirage must keep `/spider-rt` bootstrap runtime and QFS/rootfs separation.
* Zinnia's scheduler wholesale; Mirage MTSS remains the portable scheduler/process layer and Supervisor remains policy owner.
* Zinnia's driver module model wholesale; Mirage prefers supervised driver services where possible.

## Concrete implementation plan

Implemented in this patch:

1. Strengthen Mirage ELF validation with PT_LOAD page-range overlap rejection before PID1 admission.
2. Correct the initial PID1 argv path to the real `/spider-rt/sbin/spider-rs` bootstrap path and ensure stack metadata does not underflow below the mapped stack bottom after alignment.
3. Add an explicit RuntimeVfs required-layout validator that proves the manifest entry is flagged as entry and that both `spider-rs` and `spider-rsd` are discoverable through mounted `/spider-rt` paths.
4. Document the Zinnia comparison and external reference contract.

## Files changed in Mirage

* `AGENTS.md`
* `src/kernel/boot_runtime.rs`
* `src/kernel/userspace/abi.rs`
* `src/kernel/userspace/elf_loader.rs`
* `docs/audits/zinnia-reference-audit.md`
* `docs/architecture/mirage-vs-zinnia.md`
* `docs/kernel/mtss.md`
* `docs/boot/pid1-handoff.md`

## Tests run

See final report and PR body for command results. The important targeted checks are `cargo test -p mirage-kernel boot_runtime`, `cargo test -p mirage-kernel userspace::elf_loader`, and `cargo test -p mirage-kernel userspace::abi`.

## Remaining gaps

* Full ring-3 transition remains pending; Mirage must stop at `PID1 RUNNABLE` or explicit pending status until architecture entry is implemented and tested.
* Spider-rs still needs real Mirage syscall ABI wiring for spawn/exec/wait/open/read/write beyond host diagnostic stubs.
* QEMU acceptance depends on current toolchain and image build availability in the local environment.
