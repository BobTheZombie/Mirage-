use std::cell::RefCell;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pid(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnError {
    StubFailure(String),
    AbiUnavailable(&'static str),
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StubFailure(message) => write!(f, "stub spawn failure: {message}"),
            Self::AbiUnavailable(message) => write!(f, "Mirage process ABI unavailable: {message}"),
        }
    }
}

impl std::error::Error for SpawnError {}

pub trait ProcessSpawner {
    fn spawn(&self, path: &str, args: &[&str], env: &[(&str, &str)]) -> Result<Pid, SpawnError>;
    fn is_stub(&self) -> bool {
        false
    }
}

#[derive(Debug, Default)]
pub struct StubSpawner {
    log: RefCell<Vec<String>>,
}

impl StubSpawner {
    pub fn entries(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

impl ProcessSpawner for StubSpawner {
    fn spawn(&self, path: &str, args: &[&str], env: &[(&str, &str)]) -> Result<Pid, SpawnError> {
        let rendered_args = if args.is_empty() {
            String::new()
        } else {
            format!(" {}", args.join(" "))
        };
        self.log.borrow_mut().push(format!(
            "stub spawn: {path}{rendered_args} ({} env vars)",
            env.len()
        ));
        Ok(Pid(0))
    }

    fn is_stub(&self) -> bool {
        true
    }
}

/// Placeholder for the future Mirage process/syscall implementation.
#[derive(Debug, Default)]
pub struct MirageSpawner;

impl ProcessSpawner for MirageSpawner {
    fn spawn(&self, _path: &str, _args: &[&str], _env: &[(&str, &str)]) -> Result<Pid, SpawnError> {
        Err(SpawnError::AbiUnavailable(
            "spawn/execve, waitpid, stdio, and rootfs process ABI are not wired yet",
        ))
    }
}
