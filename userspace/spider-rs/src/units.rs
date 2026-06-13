#[cfg(all(feature = "host-tools", not(target_os = "none")))]
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
    #[cfg(all(feature = "host-tools", not(target_os = "none")))]
    Path,
}

impl UnitKind {
    #[cfg(all(feature = "host-tools", not(target_os = "none")))]
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

#[cfg(all(feature = "host-tools", not(target_os = "none")))]
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
    Inactive,
    Activating,
    Active,
    Failed,
    Stopping,
    #[cfg(all(feature = "host-tools", not(target_os = "none")))]
    Loaded,
    #[cfg(all(feature = "host-tools", not(target_os = "none")))]
    Stub,
    #[cfg(all(feature = "host-tools", not(target_os = "none")))]
    Skipped,
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
const BASIC_WANTS: &[&str] = &["spider-init.service"];
const DEFAULT_AFTER: &[&str] = &["basic.target"];
const DEFAULT_REQUIRES: &[&str] = &["basic.target"];

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
        wants: EMPTY,
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

#[cfg(all(feature = "host-tools", not(target_os = "none")))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestartPolicy {
    No,
    OnFailure,
    Always,
}
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
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
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
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
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceUnit {
    pub exec_start: String,
    pub restart: RestartPolicy,
}
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedUnit {
    pub unit: Unit,
    pub service: Option<ServiceUnit>,
    pub state: UnitState,
}
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
impl LoadedUnit {
    pub fn new(unit: Unit, service: Option<ServiceUnit>) -> Self {
        Self {
            unit,
            service,
            state: UnitState::Loaded,
        }
    }
}
