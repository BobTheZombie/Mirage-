#![no_std]

//! Shared ABI definitions for GNU/Mirage kernel and userspace.
//!
//! This crate is intentionally `no_std` so the kernel, Spider-rs, libc shims,
//! and host-side tests can all consume one append-only ABI table.

pub mod syscall;
