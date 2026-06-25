use core::fmt;

#[cfg(all(
    feature = "host-tests",
    not(any(target_os = "mirage", target_os = "none"))
))]
use std::cell::RefCell;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pid(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnError {
    #[cfg(all(
        feature = "host-tests",
        not(any(target_os = "mirage", target_os = "none"))
    ))]
    StubFailure(String),
    SyscallFailed(isize),
    InvalidPid(isize),
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(all(
                feature = "host-tests",
                not(any(target_os = "mirage", target_os = "none"))
            ))]
            Self::StubFailure(message) => write!(f, "stub spawn failure: {message}"),
            Self::SyscallFailed(code) => write!(f, "Mirage spawn syscall failed: {code}"),
            Self::InvalidPid(pid) => write!(f, "Mirage spawn syscall returned invalid PID: {pid}"),
        }
    }
}

#[cfg(all(
    feature = "host-tests",
    not(any(target_os = "mirage", target_os = "none"))
))]
impl std::error::Error for SpawnError {}

pub trait ProcessSpawner {
    fn spawn(&self, path: &str, args: &[&str], env: &[(&str, &str)]) -> Result<Pid, SpawnError>;
    fn is_stub(&self) -> bool {
        false
    }
}

#[cfg(all(
    feature = "host-tests",
    not(any(target_os = "mirage", target_os = "none"))
))]
#[derive(Debug, Default)]
pub struct StubSpawner {
    log: RefCell<Vec<String>>,
}

#[cfg(all(
    feature = "host-tests",
    not(any(target_os = "mirage", target_os = "none"))
))]
impl StubSpawner {
    pub fn entries(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

#[cfg(all(
    feature = "host-tests",
    not(any(target_os = "mirage", target_os = "none"))
))]
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

/// Mirage process spawner backed by the userspace spawn syscall wrapper.
#[derive(Debug, Default)]
pub struct MirageSpawner;

impl ProcessSpawner for MirageSpawner {
    fn spawn(&self, path: &str, args: &[&str], env: &[(&str, &str)]) -> Result<Pid, SpawnError> {
        match crate::syscall::spawn(path, args, env) {
            Ok(pid) if pid > 0 && pid <= u32::MAX as isize => Ok(Pid(pid as u32)),
            Ok(pid) => Err(SpawnError::InvalidPid(pid)),
            Err(code) => Err(SpawnError::SyscallFailed(code)),
        }
    }
}
