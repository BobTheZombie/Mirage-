# QEMU PID1 Boot Audit

## Observed state before this patch

- Exact stale framebuffer phase: `CURRENT PHASE: FRAMEBUFFER`, `BOOT PROGRESS: 27%`.
- Exact last reported serial marker from the failing run: `[bootdiag] device-manager install_core_devices_with_boot_info.co`, a truncated diagnostic emitted while applying BootInfo and reinstalling the logical core device table.
- Suspected blocker: inside or immediately after `kernel.bootstrap_with_boot_info(&boot_info)`, specifically `install_core_devices_with_boot_info` after graphics configuration.

## Audit findings

- `src/main.rs` enters BootInfoApplied and calls `kernel.bootstrap_with_boot_info(&boot_info)` before supervisor, boot runtime, rootfs, MTSS, and PID1 handoff.
- `src/kernel/mod.rs` resets kernel state and calls `DeviceManager::install_core_devices_with_boot_info` from the BootInfoApplied path.
- `src/kernel/device.rs` configured framebuffer and GPU capability before continuing to the post-graphics logical driver registration path.
- `src/kernel/boot_phase.rs` already keeps the boot UI display-only; the stale phase was caused by missing durable repaint/diagnostic evidence after the BootInfoApplied continuation entered.
- `src/kernel/kso/state.rs` contains honest PID1 handoff gating. It can create and mark PID1 runnable only after rootfs, supervisor, boot runtime, userspace loader, and MTSS scheduler prerequisites are satisfied. Ring-3 entry remains explicitly pending.
- `src/main.rs` has full boot code gated by `full-boot`; non-full-boot must not be used as evidence that PID1 can run.

## Current answers required by the audit

- QEMU build has full-boot enabled only when `full-boot` is present in the feature list. This patch adds `make qemu-kernel-full` and `MIRAGE_FULL_BOOT=1 make qemu-kernel` for explicit PID1 testing.
- `bootstrap_with_boot_info` return is now directly observable with `[bootflow 3.9] boot_info_applied: bootstrap_with_boot_info returned ok`.
- `install_core_devices_with_boot_info` return is now directly observable with `[bootflow 3.8] device-manager: install_core_devices_with_boot_info ok` or an exact failed/skipped reason.
- `boot_info_applied ok` is reached only after the bootstrap return marker.
- Boot runtime is found by `find_boot_runtime_module` and mounted by `BootRuntimeRamFs::mount`; failures print exact runtime blockers.
- Rootfs mount is handled by KSO `rootfs_mount`; rootfs absence blocks PID1 with a pending reason.
- Supervisor initialization is handled by KSO `supervisor_start`; supervisor absence blocks PID1 with a pending reason.
- MTSS scheduler-ready is reached by `kernel_mtss_init`; cooperative MTSS may allow PID1 when policy permits.
- PID1 launch is attempted by `maybe_retry_pid1_handoff_after_mtss_change` after MTSS readiness changes.
- PID1 is created only through `launch_spider_rs_pid1_checked` after preconditions are satisfied.
- PID1 is runnable only when the launch report says MTSS accepted the thread into a run queue.
- Ring3/user-mode transition is not completed by this patch; the kernel must print `SPIDER-RS PID1 [PENDING: ring3 transition not implemented]` rather than `RUNNING`.

## Exact blockers remaining

1. Full user-mode entry for `spider-rs` remains pending: ELF validation and PID1 runnable creation exist, but the documented ring-3 transition and first userspace syscall proof are not implemented here.
2. The framebuffer is display-only and may lag during continuation edges; serial bootflow remains source of truth. The UI must refresh on durable phases after the edge returns.
