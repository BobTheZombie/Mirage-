#![cfg_attr(all(not(test), target_os = "none"), no_std)]

//! Spider-rs userspace PID 1 service manager for GNU/Mirage.
//!
//! Spider-rs is intentionally userspace. The Supervisor authorizes its launch,
//! the userspace ELF loader validates/maps it, and MTSS schedules it as PID 1.
//! The kernel must never call Spider-rs as a Rust function.

pub mod log;
pub mod service;
pub mod start;
pub mod syscall;
pub mod target;
pub mod units;

#[cfg(all(feature = "host-tools", not(target_os = "none")))]
pub mod graph;
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
pub mod manager;
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
pub mod parser;
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
pub mod process;

#[cfg(all(feature = "host-tools", not(target_os = "none")))]
pub use graph::{DependencyError, StartupPlan};
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
pub use manager::{ServiceOutcome, SpiderManager};
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
pub use parser::{parse_unit, UnitParseError};
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
pub use process::{Pid, ProcessSpawner, SpawnError, StubSpawner};

pub use units::{default_units, UnitDescriptor, UnitKind, UnitState};
