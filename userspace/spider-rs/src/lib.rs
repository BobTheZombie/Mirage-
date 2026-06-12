//! Spider-rs userspace init/service manager for GNU/Mirage.
//!
//! Spider-rs is intentionally a userspace crate. It is not seed-rs, not a
//! kernel subsystem dispatcher, and not the kernel Supervisor. The current
//! implementation is a host-buildable scaffold that parses Mirage-native
//! `*.spider` unit files, resolves a deterministic service graph, and starts
//! services through a pluggable process-spawner abstraction.

pub mod graph;
pub mod manager;
pub mod parser;
pub mod process;
pub mod start;
pub mod syscall;
pub mod units;

pub use graph::{DependencyError, StartupPlan};
pub use manager::{ServiceOutcome, SpiderManager};
pub use parser::{parse_unit, UnitParseError};
pub use process::{Pid, ProcessSpawner, SpawnError, StubSpawner};
pub use units::{LoadedUnit, RestartPolicy, ServiceUnit, Unit, UnitKind, UnitState};
