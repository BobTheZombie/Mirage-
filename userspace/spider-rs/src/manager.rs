use crate::graph::{resolve_startup_order, DependencyError, StartupPlan};
use crate::parser::{parse_unit, UnitParseError};
use crate::process::{ProcessSpawner, SpawnError};
use crate::units::{LoadedUnit, UnitKind, UnitState};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

pub const DEFAULT_TARGET: &str = "default.target";
pub const UNIT_SEARCH_PATHS: [&str; 3] = [
    "/etc/spider/system/",
    "/usr/lib/spider/system/",
    "/run/spider/system/",
];

const BUILTIN_UNITS: [(&str, &str); 5] = [
    ("basic.target", include_str!("../units/basic.target")),
    (
        "multi-user.target",
        include_str!("../units/multi-user.target"),
    ),
    ("default.target", include_str!("../units/default.target")),
    ("shell.service", include_str!("../units/shell.service")),
    ("getty.service", include_str!("../units/getty.service")),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceOutcome {
    pub name: String,
    pub state: UnitState,
    pub message: String,
}

#[derive(Debug)]
pub enum SpiderError {
    Io(io::Error),
    Parse { name: String, error: UnitParseError },
    Dependency(DependencyError),
    Spawn { name: String, error: SpawnError },
}

impl From<DependencyError> for SpiderError {
    fn from(value: DependencyError) -> Self {
        Self::Dependency(value)
    }
}

#[derive(Clone, Debug, Default)]
pub struct SpiderManager {
    pub units: BTreeMap<String, LoadedUnit>,
}

impl SpiderManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtin_units() -> Result<Self, SpiderError> {
        let mut manager = Self::new();
        manager.load_builtin_units()?;
        Ok(manager)
    }

    pub fn load_builtin_units(&mut self) -> Result<(), SpiderError> {
        for (name, source) in BUILTIN_UNITS {
            self.insert_parsed(name, source)?;
        }
        Ok(())
    }

    pub fn load_search_paths(&mut self) -> Result<usize, SpiderError> {
        let mut loaded = 0;
        for path in UNIT_SEARCH_PATHS {
            loaded += self.load_directory(path)?;
        }
        if loaded == 0 {
            self.load_builtin_units()?;
            loaded = BUILTIN_UNITS.len();
        }
        Ok(loaded)
    }

    pub fn load_directory(&mut self, path: impl AsRef<Path>) -> Result<usize, SpiderError> {
        let path = path.as_ref();
        let Ok(entries) = fs::read_dir(path) else {
            return Ok(0);
        };
        let mut loaded = 0;
        for entry in entries {
            let entry = entry.map_err(SpiderError::Io)?;
            let file_type = entry.file_type().map_err(SpiderError::Io)?;
            if !file_type.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !is_spider_unit_name(&name) {
                continue;
            }
            let source = fs::read_to_string(entry.path()).map_err(SpiderError::Io)?;
            self.insert_parsed(&name, &source)?;
            loaded += 1;
        }
        Ok(loaded)
    }

    pub fn insert_parsed(&mut self, name: &str, source: &str) -> Result<(), SpiderError> {
        let loaded = parse_unit(name, source).map_err(|error| SpiderError::Parse {
            name: name.to_string(),
            error,
        })?;
        self.units.insert(name.to_string(), loaded);
        Ok(())
    }

    pub fn resolve_default(&self) -> Result<StartupPlan, SpiderError> {
        Ok(resolve_startup_order(&self.units, DEFAULT_TARGET)?)
    }

    pub fn start_plan<S: ProcessSpawner>(
        &mut self,
        plan: &StartupPlan,
        spawner: &S,
    ) -> Vec<ServiceOutcome> {
        let mut outcomes = Vec::new();
        for name in &plan.order {
            let blocked = self.required_dependency_failed(name);
            if let Some(failed_dep) = blocked {
                if let Some(unit) = self.units.get_mut(name) {
                    unit.state = UnitState::Failed;
                }
                outcomes.push(ServiceOutcome {
                    name: name.clone(),
                    state: UnitState::Failed,
                    message: format!("required dependency failed: {failed_dep}"),
                });
                continue;
            }

            let (state, message) = match self.units.get(name) {
                Some(loaded) if loaded.unit.kind == UnitKind::Target => {
                    (UnitState::Active, "target reached".to_string())
                }
                Some(loaded) if loaded.unit.kind == UnitKind::Service => {
                    match loaded.service.as_ref() {
                        Some(service) => start_service(service.exec_start.as_str(), spawner),
                        None => (
                            UnitState::Failed,
                            "service unit has no [Service] data".to_string(),
                        ),
                    }
                }
                Some(loaded) => (
                    UnitState::Skipped,
                    format!(
                        "{} units are reserved for a later Spider-rs milestone",
                        loaded.unit.kind
                    ),
                ),
                None => (
                    UnitState::Failed,
                    "unit disappeared during startup".to_string(),
                ),
            };

            if let Some(unit) = self.units.get_mut(name) {
                unit.state = state.clone();
            }
            outcomes.push(ServiceOutcome {
                name: name.clone(),
                state,
                message,
            });
        }
        outcomes
    }

    fn required_dependency_failed(&self, name: &str) -> Option<String> {
        let unit = self.units.get(name)?;
        unit.unit.requires.iter().find_map(|required| {
            self.units
                .get(required)
                .and_then(|loaded| (loaded.state == UnitState::Failed).then(|| required.clone()))
        })
    }
}

fn start_service<S: ProcessSpawner>(exec_start: &str, spawner: &S) -> (UnitState, String) {
    let argv = split_exec_start(exec_start);
    let Some((path, args)) = argv.split_first() else {
        return (UnitState::Failed, "empty ExecStart".to_string());
    };
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match spawner.spawn(path, &refs, &[]) {
        Ok(pid) if spawner.is_stub() => (
            UnitState::Stub,
            format!(
                "stubbed ExecStart={exec_start}; no process executed (pid token {:?})",
                pid
            ),
        ),
        Ok(pid) => (UnitState::Active, format!("started with pid {}", pid.0)),
        Err(error) => (UnitState::Failed, error.to_string()),
    }
}

fn split_exec_start(exec_start: &str) -> Vec<String> {
    exec_start.split_whitespace().map(str::to_string).collect()
}

fn is_spider_unit_name(name: &str) -> bool {
    matches!(
        name.rsplit_once('.').map(|(_, suffix)| suffix),
        Some("service" | "target" | "socket" | "timer" | "mount" | "device" | "path" | "spider")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{Pid, ProcessSpawner, SpawnError, StubSpawner};

    #[derive(Default)]
    struct FailingSpawner;

    impl ProcessSpawner for FailingSpawner {
        fn spawn(
            &self,
            _path: &str,
            _args: &[&str],
            _env: &[(&str, &str)],
        ) -> Result<Pid, SpawnError> {
            Err(SpawnError::StubFailure(
                "intentional test failure".to_string(),
            ))
        }
    }

    #[test]
    fn stub_spawner_marks_service_stub() {
        let mut manager = SpiderManager::new();
        manager
            .insert_parsed(
                "default.target",
                "[Unit]\nRequires=shell.service\nAfter=shell.service\n",
            )
            .unwrap();
        manager
            .insert_parsed(
                "shell.service",
                "[Unit]\nDescription=Shell\n[Service]\nExecStart=/bin/msh\n",
            )
            .unwrap();
        let plan = manager.resolve_default().unwrap();
        let spawner = StubSpawner::default();
        let outcomes = manager.start_plan(&plan, &spawner);
        assert!(outcomes
            .iter()
            .any(|outcome| outcome.name == "shell.service" && outcome.state == UnitState::Stub));
        assert_eq!(spawner.entries().len(), 1);
    }

    #[test]
    fn requires_failure_propagates() {
        let mut manager = SpiderManager::new();
        manager
            .insert_parsed(
                "default.target",
                "[Unit]\nRequires=bad.service\nAfter=bad.service\n",
            )
            .unwrap();
        manager
            .insert_parsed(
                "bad.service",
                "[Unit]\nDescription=Bad\n[Service]\nExecStart=/bin/bad\n",
            )
            .unwrap();
        let plan = manager.resolve_default().unwrap();
        let outcomes = manager.start_plan(&plan, &FailingSpawner);
        assert!(outcomes
            .iter()
            .any(|outcome| outcome.name == "bad.service" && outcome.state == UnitState::Failed));
        assert!(outcomes
            .iter()
            .any(|outcome| outcome.name == "default.target" && outcome.state == UnitState::Failed));
    }

    #[test]
    fn wants_failure_is_non_fatal() {
        let mut manager = SpiderManager::new();
        manager
            .insert_parsed(
                "default.target",
                "[Unit]\nWants=optional.service\nAfter=optional.service\n",
            )
            .unwrap();
        manager
            .insert_parsed(
                "optional.service",
                "[Unit]\nDescription=Optional\n[Service]\nExecStart=/bin/optional\n",
            )
            .unwrap();
        let plan = manager.resolve_default().unwrap();
        let outcomes = manager.start_plan(&plan, &FailingSpawner);
        assert!(outcomes.iter().any(
            |outcome| outcome.name == "optional.service" && outcome.state == UnitState::Failed
        ));
        assert!(outcomes
            .iter()
            .any(|outcome| outcome.name == "default.target" && outcome.state == UnitState::Active));
    }
}
