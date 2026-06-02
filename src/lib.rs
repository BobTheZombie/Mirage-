#![cfg_attr(not(test), no_std)]

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

#[cfg(feature = "qfs-std")]
extern crate std;

pub mod arch;
pub mod boot;
pub mod kernel;
#[cfg(not(feature = "qfs-std"))]
pub mod libc;
#[cfg(not(feature = "qfs-std"))]
pub mod librust;
#[cfg(not(feature = "qfs-std"))]
pub mod stdlib;
pub mod subkernel;

#[cfg(not(any(test, feature = "qfs-std")))]
use core::panic::PanicInfo;

#[cfg(not(any(test, feature = "qfs-std")))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    crate::arch::x86_64::panic_halt()
}
