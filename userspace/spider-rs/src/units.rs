use std::fmt;

/// Unit kinds understood or reserved by Spider-rs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnitKind {
    Service,
    Target,
    Socket,
    Timer,
    Mount,
    Device,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitState {
    Loaded,
    Inactive,
    Activating,
    Active,
    /// Current milestone used StubSpawner; no Mirage process was executed.
    Stub,
    Failed,
    Skipped,
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
