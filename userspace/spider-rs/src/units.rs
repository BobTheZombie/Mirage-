#[cfg(target_os = "none")]
use alloc::{string::String, vec::Vec};
#[cfg(all(feature = "host-tests", not(target_os = "none")))]
use std::fmt;

/// Unit kinds understood by Spider-rs v0 and reserved for later milestones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnitKind {
    Service,
    Target,
    Mount,
    Device,
    Timer,
    Socket,
    Path,
}

impl UnitKind {
    pub fn from_name(name: &str) -> Option<Self> {
        let suffix = name.rsplit_once('.').map(|(_, suffix)| suffix)?;
        match suffix {
            "service" => Some(Self::Service),
            "target" => Some(Self::Target),
            "socket" => Some(Self::Socket),
            "timer" => Some(Self::Timer),
            "mount" => Some(Self::Mount),
            "device" => Some(Self::Device),
            "path" => Some(Self::Path),
            _ => None,
        }
    }
}

#[cfg(all(feature = "host-tests", not(target_os = "none")))]
impl fmt::Display for UnitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Service => "service",
            Self::Target => "target",
            Self::Socket => "socket",
            Self::Timer => "timer",
            Self::Mount => "mount",
            Self::Device => "device",
            Self::Path => "path",
        };
        f.write_str(text)
    }
}

/// Runtime state tracked by Spider-rs for a loaded unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitState {
    Loaded,
    Waiting,
    Starting,
    Running,
    Exited,
    Failed,
    Skipped,
    Inactive,
    Activating,
    Active,
    Stopping,
}

/// Static v0 unit descriptor. Dynamic parsers can later populate equivalent records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitDescriptor {
    pub name: &'static str,
    pub kind: UnitKind,
    pub description: &'static str,
    pub after: &'static [&'static str],
    pub requires: &'static [&'static str],
    pub wants: &'static [&'static str],
}

const EMPTY: &[&str] = &[];
const BASIC_WANTS: &[&str] = &[];
const DEFAULT_AFTER: &[&str] = &["basic.target"];
const DEFAULT_REQUIRES: &[&str] = &[];
const DEFAULT_WANTS: &[&str] = &["basic.target", "m1-terminal.service"];
const M1_AFTER: &[&str] = &["basic.target"];
const M1_WANTS: &[&str] = &["basic.target"];

pub static BUILTIN_UNITS: &[UnitDescriptor] = &[
    UnitDescriptor {
        name: "basic.target",
        kind: UnitKind::Target,
        description: "Basic Spider userspace target",
        after: EMPTY,
        requires: EMPTY,
        wants: BASIC_WANTS,
    },
    UnitDescriptor {
        name: "default.target",
        kind: UnitKind::Target,
        description: "Default Mirage userspace target",
        after: DEFAULT_AFTER,
        requires: DEFAULT_REQUIRES,
        wants: DEFAULT_WANTS,
    },
    UnitDescriptor {
        name: "m1-terminal.service",
        kind: UnitKind::Service,
        description: "Mirage M1.1 first terminal service",
        after: M1_AFTER,
        requires: EMPTY,
        wants: M1_WANTS,
    },
    UnitDescriptor {
        name: "emergency.target",
        kind: UnitKind::Target,
        description: "Emergency Spider shell target placeholder",
        after: EMPTY,
        requires: EMPTY,
        wants: EMPTY,
    },
    UnitDescriptor {
        name: "spider-init.service",
        kind: UnitKind::Service,
        description: "Spider first-stage initialization hooks",
        after: EMPTY,
        requires: EMPTY,
        wants: EMPTY,
    },
];

pub fn default_units() -> &'static [UnitDescriptor] {
    BUILTIN_UNITS
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestartPolicy {
    No,
    OnFailure,
    Always,
}
impl RestartPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "no" | "No" => Some(Self::No),
            "on-failure" | "OnFailure" | "on_failure" => Some(Self::OnFailure),
            "always" | "Always" => Some(Self::Always),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub name: String,
    pub description: String,
    pub kind: UnitKind,
    pub after: Vec<String>,
    pub before: Vec<String>,
    pub requires: Vec<String>,
    pub wants: Vec<String>,
    pub wanted_by: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceUnit {
    pub exec_start: String,
    pub restart: RestartPolicy,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedUnit {
    pub unit: Unit,
    pub service: Option<ServiceUnit>,
    pub state: UnitState,
}
impl LoadedUnit {
    pub fn new(unit: Unit, service: Option<ServiceUnit>) -> Self {
        Self {
            unit,
            service,
            state: UnitState::Loaded,
        }
    }
}
