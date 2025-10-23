#![no_std]

//! Mirage is a conceptual 64-bit, Rust-based kernel split into two cooperative layers.
//!
//! * The **L1 core** (the _main kernel_) is responsible for CPU scheduling, process lifecycle
//!   management and inter-process communication primitives that resemble a traditional
//!   Unix-like microkernel.
//! * The **L2 security core** (the _sub-kernel_) enforces isolation domains and authenticates
//!   every task and message before they interact with the core services.
//!
//! The code in this crate is designed to illustrate the internal structure of such a kernel
//! without relying on the standard library. While the implementation is intentionally lean,
//! it captures the essential mechanics one would expect from a Linux-like 64-bit kernel
//! written in Rust.

pub mod arch;
pub mod kernel;
pub mod subkernel;
