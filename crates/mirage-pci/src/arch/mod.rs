//! Architecture-specific PCI configuration access backends.

#[cfg(target_arch = "x86_64")]
pub mod x86_64;
