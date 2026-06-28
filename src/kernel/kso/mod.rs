//! Kernel Service Object (KSO) boot graph support.
//!
//! KSO is deliberately small and `no_std` safe: it stores static dependency
//! graph slices, checks kernel-enforced capability prerequisites, and reports
//! truthful state. Service launch policy remains with the Mirage Supervisor.

pub mod generated;
pub mod graph;
pub mod policy;
pub mod runner;
pub mod state;

pub use graph::{KsoNode, KsoNodeRuntime};
pub use policy::{KsoCapability, KsoFailurePolicy, KsoNodeKind, KsoPolicy, KsoStartupFnId};
pub use runner::{KsoRunner, KsoStartFn};
pub use state::{
    kso_transition, maybe_retry_pid1_handoff_after_mtss_change, BootContinueResult,
    BootRuntimeDeps, KsoBootNode, KsoContext, KsoNodeId, KsoRunOutcome, KsoStartResult, KsoState,
    KsoStatus, MtssReadiness,
};

#[cfg(test)]
mod tests {
    use super::*;

    const MTSS: KsoNodeId = KsoNodeId(1);
    const TERM: KsoNodeId = KsoNodeId(2);
    const SCHED_CAP: KsoCapability = KsoCapability("mtss.scheduler");

    fn ok(_: KsoStartupFnId) -> KsoStartResult {
        KsoStartResult::Started
    }
    fn fail(_: KsoStartupFnId) -> KsoStartResult {
        KsoStartResult::Failed
    }

    #[test]
    fn reports_capability_blocker_and_retries_after_grant() {
        let nodes = [KsoNode {
            id: TERM,
            name: "terminal",
            kind: KsoNodeKind::Application,
            startup: KsoStartupFnId(1),
            after: &[],
            wants: &[],
            requires: &[SCHED_CAP],
            wants_capabilities: &[],
            provides: &[],
            optional_provides: &[],
            policy: KsoPolicy::REQUIRED,
        }];
        let mut runtime = [KsoNodeRuntime::new()];
        let mut caps = [None; 2];
        let mut runner = KsoRunner::new(&nodes, &mut runtime, &mut caps);
        assert_eq!(runner.run_once(ok), KsoRunOutcome::Blocked);
        let status = runner.status(TERM).unwrap();
        assert_eq!(status.blocker, Some("mtss.scheduler"));
        let mut message = [0u8; 32];
        assert_eq!(
            status.blocker_message(&mut message),
            Some("waiting for: mtss.scheduler")
        );
        assert!(runner.grant_capability(SCHED_CAP));
        assert_eq!(runner.run_once(ok), KsoRunOutcome::Complete);
        assert_eq!(runner.status(TERM).unwrap().state, KsoState::Online);
    }

    #[test]
    fn hard_after_dependency_orders_startup() {
        let nodes = [
            KsoNode {
                id: TERM,
                name: "terminal",
                kind: KsoNodeKind::Application,
                startup: KsoStartupFnId(2),
                after: &[MTSS],
                wants: &[],
                requires: &[],
                wants_capabilities: &[],
                provides: &[],
                optional_provides: &[],
                policy: KsoPolicy::REQUIRED,
            },
            KsoNode {
                id: MTSS,
                name: "mtss.scheduler",
                kind: KsoNodeKind::MtssMechanism,
                startup: KsoStartupFnId(1),
                after: &[],
                wants: &[],
                requires: &[],
                wants_capabilities: &[],
                provides: &[],
                optional_provides: &[],
                policy: KsoPolicy::REQUIRED,
            },
        ];
        let mut runtime = [KsoNodeRuntime::new(), KsoNodeRuntime::new()];
        let mut caps = [None; 1];
        let mut runner = KsoRunner::new(&nodes, &mut runtime, &mut caps);
        assert_eq!(runner.run_once(ok), KsoRunOutcome::Progress);
        assert_eq!(runner.status(TERM).unwrap().blocker, Some("mtss.scheduler"));
        assert_eq!(runner.run_once(ok), KsoRunOutcome::Complete);
    }

    #[test]
    fn required_failure_is_fatal_unless_degraded_allowed() {
        let nodes = [KsoNode {
            id: MTSS,
            name: "mtss.scheduler",
            kind: KsoNodeKind::MtssMechanism,
            startup: KsoStartupFnId(1),
            after: &[],
            wants: &[],
            requires: &[],
            wants_capabilities: &[],
            provides: &[],
            optional_provides: &[],
            policy: KsoPolicy::REQUIRED,
        }];
        let mut runtime = [KsoNodeRuntime::new()];
        let mut caps = [None; 1];
        let mut runner = KsoRunner::new(&nodes, &mut runtime, &mut caps);
        assert_eq!(
            runner.run_once(fail),
            KsoRunOutcome::Fatal {
                node: MTSS,
                reason: "required node failed"
            }
        );
    }

    #[test]
    fn optional_driver_failure_policy_is_non_fatal() {
        let nodes = [KsoNode {
            id: KsoNodeId(3),
            name: "usbd",
            kind: KsoNodeKind::DriverService,
            startup: KsoStartupFnId(3),
            after: &[],
            wants: &[MTSS],
            requires: &[],
            wants_capabilities: &[],
            provides: &[],
            optional_provides: &[],
            policy: KsoPolicy::OPTIONAL_DRIVER,
        }];
        let mut runtime = [KsoNodeRuntime::new()];
        let mut caps = [None; 1];
        let mut runner = KsoRunner::new(&nodes, &mut runtime, &mut caps);
        assert_eq!(runner.run_once(fail), KsoRunOutcome::Complete);
        assert_eq!(
            runner.status(KsoNodeId(3)).unwrap().state,
            KsoState::Degraded
        );
    }
}
