//! Static KSO graph representation.

use super::policy::{KsoCapability, KsoNodeKind, KsoPolicy, KsoStartupFnId};
use super::state::{KsoNodeId, KsoState};

/// One node in the KSO dependency graph. All references are static slices so the
/// graph is safe for `no_std` target images and deterministic boot ordering.
#[derive(Clone, Copy, Debug)]
pub struct KsoNode {
    pub id: KsoNodeId,
    pub name: &'static str,
    pub kind: KsoNodeKind,
    pub startup: KsoStartupFnId,
    pub after: &'static [KsoNodeId],
    pub wants: &'static [KsoNodeId],
    pub requires: &'static [KsoCapability],
    pub provides: &'static [KsoCapability],
    pub policy: KsoPolicy,
}

#[derive(Clone, Copy, Debug)]
pub struct KsoNodeRuntime {
    pub state: KsoState,
    pub blocker: Option<&'static str>,
}

impl KsoNodeRuntime {
    pub const fn new() -> Self {
        Self {
            state: KsoState::New,
            blocker: None,
        }
    }
}

impl Default for KsoNodeRuntime {
    fn default() -> Self {
        Self::new()
    }
}
