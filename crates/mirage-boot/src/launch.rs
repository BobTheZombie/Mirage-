use crate::manifest::{
    BootModuleCapabilityRequest, BootModuleId, BootModuleKind, BootModuleManifest, RestartPolicy,
};
use crate::policy::{decide_module_policy, BootModulePolicyDecision, PolicyRejectionReason};
use crate::signature::verify_mock_signature;

/// Launch plan for supervisor-managed boot services.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceLaunchPlan {
    pub module_id: BootModuleId,
    pub kind: BootModuleKind,
    pub image: String,
    pub restart: RestartPolicy,
    pub capabilities: Vec<BootModuleCapabilityRequest>,
}

/// Errors that stop service launch planning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchPlanError {
    PolicyDenied {
        module_id: BootModuleId,
        reason: PolicyRejectionReason,
    },
}

/// Build service launch inputs after parsing, signature validation, and policy admission.
pub fn build_service_launch_plan(
    manifest: &BootModuleManifest,
) -> Result<Vec<ServiceLaunchPlan>, LaunchPlanError> {
    let mut plans = Vec::new();

    for module in &manifest.modules {
        let validation = verify_mock_signature(module);
        match decide_module_policy(module, &validation) {
            BootModulePolicyDecision::Allow { capabilities, .. } => {
                if is_launchable_service(module.kind) {
                    plans.push(ServiceLaunchPlan {
                        module_id: module.id.clone(),
                        kind: module.kind,
                        image: module.image.clone(),
                        restart: module.restart,
                        capabilities,
                    });
                }
            }
            BootModulePolicyDecision::Deny { module_id, reason } => {
                return Err(LaunchPlanError::PolicyDenied { module_id, reason });
            }
        }
    }

    Ok(plans)
}

fn is_launchable_service(kind: BootModuleKind) -> bool {
    matches!(
        kind,
        BootModuleKind::Service
            | BootModuleKind::DriverService
            | BootModuleKind::Filesystem
            | BootModuleKind::Runtime
    )
}
