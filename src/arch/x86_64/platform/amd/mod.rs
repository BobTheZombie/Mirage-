//! AMD x86_64 lower-kernel platform support.

pub mod renoir;
pub mod scheduler;

pub use renoir::{renoir_kernel_boot_probe, RenoirBootProfile, RenoirCpuidFacts};
pub use scheduler::{select_renoir_scheduler_module, RenoirSchedulerSelection};
