//! Internal service boundaries for kernel subsystems.
//!
//! These traits provide narrow adapters over the kernel subsystems so host-side
//! tests and shims can depend on kernel-facing capabilities without importing
//! user-facing `crate::stdlib` conveniences. User ABI wrappers should continue
//! to enter the kernel through the syscall ABI; in-process tests may use these
//! traits when they need direct subsystem seams.

pub mod device;
pub mod fs;
pub mod memory;
pub mod process;
pub mod time;

pub use device::DeviceService;
pub use fs::FileSystemService;
pub use memory::{KernelMemoryService, MemoryService};
pub use process::ProcessService;
pub use time::TimeService;
