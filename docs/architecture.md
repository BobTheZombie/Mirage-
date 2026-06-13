# Mirage architecture ownership

- **seed-rs** owns early Rust entry, BSS clear, linker section capture, Limine snapshot, BootInfo construction, and `kernel_main` handoff.
- **Kernel core** owns memory, paging, heap bootstrap, IDT/PIC/interrupts, platform registry, MMIO, DMA primitives, syscall entry, and capability enforcement mechanisms.
- **Mirage-dispatch-rs** owns kernel service/component registration, dependency ordering, and startup dispatch for kernel components.
- **Boot Phase Manager** reports status only. It does not own policy.
- **Supervisor** is the kernel-side policy authority. It owns RuntimeVfs policy, Spider-rs lifecycle authorization, recovery and respawn policy, and userspace launch authority.
- **MTSS** owns scheduling, task/thread execution records, userspace process execution, PID management, and the IPC/event-loop foundation.
- **RuntimeVfs** is the permanent immutable `/spider-rt` filesystem. It is kernel/Supervisor owned, read-only, memory resident, and trusted as the Spider runtime source.
- **Boot Runtime** (`/bootrt`) is temporary and optional after Spider Runtime is online.
- **Spider-rs** is userspace PID 1. It is a systemd-like service manager for targets, units, userspace service supervision, mount orchestration hooks, logs, shutdown/reboot ordering, and emergency mode.
- **Normal rootfs** (QFS/ext4/etc.) is separate from `/spider-rt` and may be mutable or read-only according to mount policy.
