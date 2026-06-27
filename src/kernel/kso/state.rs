//! KSO state and stable identifiers.

/// Stable identifier for a kernel service object node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KsoNodeId(pub u16);

/// Runtime lifecycle state tracked by the deterministic KSO runner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KsoState {
    NotStarted,
    New,
    WaitingDeps,
    Starting,
    Ready,
    Online,
    Degraded,
    Skipped,
    Disabled,
    Failed,
    Running,
}

/// Public status snapshot for a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KsoStatus {
    pub id: KsoNodeId,
    pub state: KsoState,
    pub blocker: Option<&'static str>,
}

impl KsoStatus {
    /// Writes a stable human-readable blocker such as `waiting for: mtss.scheduler`.
    pub fn blocker_message<'a>(&self, buffer: &'a mut [u8]) -> Option<&'a str> {
        let blocker = self.blocker?;
        let prefix = b"waiting for: ";
        let bytes = blocker.as_bytes();
        if buffer.len() < prefix.len() + bytes.len() {
            return None;
        }
        buffer[..prefix.len()].copy_from_slice(prefix);
        buffer[prefix.len()..prefix.len() + bytes.len()].copy_from_slice(bytes);
        core::str::from_utf8(&buffer[..prefix.len() + bytes.len()]).ok()
    }
}

/// Result returned by a node startup thunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KsoStartResult {
    Started,
    StartedDegraded,
    Skipped,
    Disabled,
    Failed,
}

/// Deterministic run result for a runner pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KsoRunOutcome {
    Complete,
    Progress,
    Blocked,
    Fatal {
        node: KsoNodeId,
        reason: &'static str,
    },
}
