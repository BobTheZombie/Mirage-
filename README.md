# Mirage Kernel

Mirage is a conceptual 64-bit, Rust-based operating system kernel organised into two tightly
coupled layers:

* **Level 1 (L1) core** – handles CPU scheduling, process lifecycle management and message-based
  inter-process communication (IPC).
* **Level 2 (L2) security core** – authenticates every task, enforces isolation domains and
  adjudicates the flow of messages between processes.

The code in this repository purposefully focuses on structure rather than device drivers or
bootstrapping logic. It is meant to showcase how the responsibilities are divided between the two
layers while staying entirely within `#![no_std]` Rust.

## Layout

```
src/
├── arch/          # 64-bit x86 architectural scaffolding (initialisation, CPU hints)
├── kernel/        # L1 kernel components: processes, scheduler, IPC queues
├── subkernel/     # L2 security kernel responsible for isolation domains and capabilities
├── lib.rs         # Crate entry point that exposes the layered kernel modules
└── main.rs        # `_start` entry point wiring the layers together
```

## Highlights

* **Pure Rust, no_std:** everything is written in Rust without the standard library to mirror a
  freestanding kernel environment.
* **Deterministic resource management:** fixed-size tables and ring buffers are used instead of
  heap allocations, making the control flow easy to audit.
* **Security-aware IPC:** every message is tagged with a security class and must be authorised by
  the L2 kernel before delivery.
* **Composable design:** the separation between the L1 core and the L2 security kernel allows
  experimentation with different scheduling policies or security models in isolation.

## Status

This implementation is intentionally minimal and is not meant to be booted on real hardware without
additional work (bootloader integration, device drivers, memory management, etc.). It acts as a
foundation for exploring ideas around microkernel design in Rust.
