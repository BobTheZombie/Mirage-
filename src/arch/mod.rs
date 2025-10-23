//! Architecture specific helpers used by the Mirage kernel.
//!
//! The current implementation targets 64-bit x86 hardware. Platform abstractions are kept
//! intentionally small to highlight the kernel layering rather than the minutiae of device
//! drivers or bootloader integration.

pub mod x86_64;
