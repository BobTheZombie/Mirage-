//! Deterministic KSO runner.

use super::graph::{KsoNode, KsoNodeRuntime};
use super::policy::{KsoCapability, KsoFailurePolicy, KsoStartupFnId};
use super::state::{KsoNodeId, KsoRunOutcome, KsoStartResult, KsoState, KsoStatus};
use crate::kernel::boot_phase::boot_phase_apply_kso_transition;

pub type KsoStartFn = fn(KsoStartupFnId) -> KsoStartResult;

pub struct KsoRunner<'a> {
    nodes: &'a [KsoNode],
    runtime: &'a mut [KsoNodeRuntime],
    caps: &'a mut [Option<KsoCapability>],
}

impl<'a> KsoRunner<'a> {
    pub fn new(
        nodes: &'a [KsoNode],
        runtime: &'a mut [KsoNodeRuntime],
        caps: &'a mut [Option<KsoCapability>],
    ) -> Self {
        assert!(nodes.len() == runtime.len());
        Self {
            nodes,
            runtime,
            caps,
        }
    }

    pub fn status(&self, id: KsoNodeId) -> Option<KsoStatus> {
        let idx = self.index_of(id)?;
        Some(KsoStatus {
            id,
            state: self.runtime[idx].state,
            blocker: self.runtime[idx].blocker,
        })
    }

    pub fn grant_capability(&mut self, cap: KsoCapability) -> bool {
        if self.has_capability(cap) {
            return true;
        }
        for slot in self.caps.iter_mut() {
            if slot.is_none() {
                *slot = Some(cap);
                self.retry_waiting_deps();
                return true;
            }
        }
        false
    }

    pub fn run_once(&mut self, start: KsoStartFn) -> KsoRunOutcome {
        let mut progress = false;
        for idx in 0..self.nodes.len() {
            if !matches!(
                self.runtime[idx].state,
                KsoState::NotStarted | KsoState::New | KsoState::WaitingDeps
            ) {
                continue;
            }
            if let Some(blocker) = self.blocker_for(idx) {
                self.set_state(idx, KsoState::WaitingDeps, "waiting for dependency");
                self.runtime[idx].blocker = Some(blocker);
                continue;
            }
            self.set_state(idx, KsoState::Starting, "starting");
            self.runtime[idx].blocker = None;
            match start(self.nodes[idx].startup) {
                KsoStartResult::Started => {
                    self.set_state(idx, KsoState::Online, "online");
                    self.publish_caps(idx);
                    progress = true;
                }
                KsoStartResult::StartedDegraded => {
                    self.set_state(idx, KsoState::Degraded, "degraded");
                    self.publish_caps(idx);
                    progress = true;
                }
                KsoStartResult::Skipped => {
                    self.set_state(idx, KsoState::Skipped, "skipped");
                    progress = true;
                }
                KsoStartResult::Disabled => {
                    self.set_state(idx, KsoState::Disabled, "disabled");
                    progress = true;
                }
                KsoStartResult::Failed => match self.failure_state(idx) {
                    Err(outcome) => return outcome,
                    Ok(state) => {
                        self.set_state(idx, state, "failure policy applied");
                        progress = true;
                    }
                },
            }
        }
        if self.complete() {
            KsoRunOutcome::Complete
        } else if progress {
            KsoRunOutcome::Progress
        } else {
            KsoRunOutcome::Blocked
        }
    }

    fn retry_waiting_deps(&mut self) {
        for rt in self.runtime.iter_mut() {
            if rt.state == KsoState::WaitingDeps {
                rt.state = KsoState::New;
                rt.blocker = None;
            }
        }
    }

    fn blocker_for(&self, idx: usize) -> Option<&'static str> {
        let node = &self.nodes[idx];
        for dep in node.after {
            let dep_idx = self.index_of(*dep)?;
            if !matches!(
                self.runtime[dep_idx].state,
                KsoState::Ready
                    | KsoState::Online
                    | KsoState::Degraded
                    | KsoState::Skipped
                    | KsoState::Disabled
            ) {
                return Some(self.nodes[dep_idx].name);
            }
        }
        for cap in node.requires {
            if !self.has_capability(*cap) {
                return Some(cap.0);
            }
        }
        if !node.policy.allow_missing_wants {
            for dep in node.wants {
                let dep_idx = self.index_of(*dep)?;
                if !matches!(
                    self.runtime[dep_idx].state,
                    KsoState::Ready
                        | KsoState::Online
                        | KsoState::Degraded
                        | KsoState::Skipped
                        | KsoState::Disabled
                ) {
                    return Some(self.nodes[dep_idx].name);
                }
            }
        }
        None
    }

    fn failure_state(&self, idx: usize) -> Result<KsoState, KsoRunOutcome> {
        let node = &self.nodes[idx];
        match (node.policy.required, node.policy.failure) {
            (true, KsoFailurePolicy::Fatal) => Err(KsoRunOutcome::Fatal {
                node: node.id,
                reason: "required node failed",
            }),
            (true, KsoFailurePolicy::AllowDegraded) => Ok(KsoState::Degraded),
            (_, KsoFailurePolicy::Skip) => Ok(KsoState::Skipped),
            (_, KsoFailurePolicy::Disable) => Ok(KsoState::Disabled),
            (_, KsoFailurePolicy::AllowDegraded) => Ok(KsoState::Degraded),
            (false, KsoFailurePolicy::Fatal | KsoFailurePolicy::MarkFailedNonFatal) => {
                Ok(KsoState::Failed)
            }
            (true, KsoFailurePolicy::MarkFailedNonFatal) => Err(KsoRunOutcome::Fatal {
                node: node.id,
                reason: "required node failed",
            }),
        }
    }

    fn set_state(&mut self, idx: usize, state: KsoState, message: &'static str) {
        self.runtime[idx].state = state;
        let _ = boot_phase_apply_kso_transition(self.nodes[idx].id, state, message);
    }

    fn publish_caps(&mut self, idx: usize) {
        for cap in self.nodes[idx].provides {
            let _ = self.grant_capability(*cap);
        }
    }

    fn has_capability(&self, cap: KsoCapability) -> bool {
        self.caps.iter().any(|slot| *slot == Some(cap))
    }
    fn index_of(&self, id: KsoNodeId) -> Option<usize> {
        self.nodes.iter().position(|node| node.id == id)
    }
    fn complete(&self) -> bool {
        self.runtime.iter().all(|rt| {
            !matches!(
                rt.state,
                KsoState::NotStarted | KsoState::New | KsoState::WaitingDeps | KsoState::Starting
            )
        })
    }
}
