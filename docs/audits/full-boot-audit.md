# Mirage full boot audit

## Starting failure state

The reported live milestone UI could park after hardware/input bring-up with `CURRENT PHASE: KERNELCONSTRUCTED`, leaving rootfs, supervisor, MTSS, userspace loader, PID1, dispatcher, terminal, userspace, and idleloop unresolved.

## Root cause(s)

The strongest local root-cause candidate is boot UI repaint coupling: every stable phase transition could synchronously repaint the framebuffer. `KernelConstructed` is an internal continuation edge, not a human-facing terminal milestone. If the framebuffer repaint path stalls at that edge, the boot pipeline does not get to `BootInfoApplied`, supervisor creation, runtime validation, rootfs, MTSS, or PID1 eligibility.

## Files inspected

Audited `src/main.rs`, `src/kernel/boot_phase.rs`, `src/kernel/boot_runtime.rs`, `src/kernel/debug_shell.rs`, `src/arch/x86_64/mod.rs`, `src/arch/x86_64/ps2_keyboard.rs`, `tools/validate-boot-runtime.sh`, QEMU runner scripts under `tools/`, and existing boot/userspace documentation.

## Files changed

- `src/main.rs`
- `src/kernel/boot_phase.rs`
- `AGENTS.md`
- `docs/boot/boot-flow.md`
- `docs/boot/boot-milestone-1.1.md`
- `docs/boot/live-milestone-ui.md`
- `docs/boot/qemu-boot.md`
- `docs/boot/virtualbox-boot.md`
- `docs/boot/pid1-handoff.md`
- `docs/kernel/mtss.md`
- `docs/kernel/mtss-readiness.md`
- `docs/audits/full-boot-audit.md`

## Boot phase graph before

`architecture init -> KernelConstructed -> framebuffer repaint on KernelConstructed OK -> apply boot info -> supervisor -> runtime/rootfs -> MTSS -> PID1 eligibility -> idleloop`

The graph had a suspected blocking UI repaint between `KernelConstructed` and `BootInfoApplied`.

## Boot phase graph after

`architecture init -> KernelConstructed -> BootInfoApplied -> SupervisorCreated -> BootRuntime -> RootFs -> Supervisor -> MTSS -> PID1 eligibility -> UserspaceLoader -> Spider-rs preflight -> PID1 task creation -> BootScreen -> IdleLoop`

`KernelConstructed` no longer forces a framebuffer repaint; it remains serial-observable and the next continuation edge is instrumented.

## Tests added

- `KernelConstructed` OK does not force a framebuffer repaint.
- Post-kernel phases remain framebuffer-visible milestones.
- Required pending phases keep progress below 100%.

## QEMU result

Not fully accepted in this environment during this patch. The code still truthfully reports ring3/userspace transition as pending when not implemented; it does not claim M1.1 100% completion.

## VirtualBox result

Not fully accepted in this environment during this patch. `VBoxManage` availability and host kernel-module access must be verified on Derek's local machine before claiming VirtualBox boot acceptance.

## Remaining limitations

- Ring3 userspace transition remains the known blocker if the target code cannot yet execute PID1.
- `m1-terminal` output must not be claimed until it flows from the actual userspace binary through the Mirage syscall path.
- Full QEMU and VirtualBox logs are still required before opening a boot-completion PR.
