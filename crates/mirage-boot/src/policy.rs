use crate::manifest::{BootModule, BootModuleCapabilityRequest, BootModuleId, BootModuleKind};
use crate::signature::{BootModuleValidationResult, ValidationFailureReason};

/// Supervisor admission decision for a validated boot module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BootModulePolicyDecision {
    Allow {
        module_id: BootModuleId,
        kind: BootModuleKind,
        capabilities: Vec<BootModuleCapabilityRequest>,
    },
    Deny {
        module_id: BootModuleId,
        reason: PolicyRejectionReason,
    },
}

/// Supervisor policy rejection reasons kept separate from signature validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyRejectionReason {
    Signature(ValidationFailureReason),
    ValidationModuleMismatch {
        expected: BootModuleId,
        actual: BootModuleId,
    },
}

/// Convert validation output plus manifest metadata into a supervisor policy decision.
pub fn decide_module_policy(
    module: &BootModule,
    validation: &BootModuleValidationResult,
) -> BootModulePolicyDecision {
    if validation.module_id() != &module.id {
        return BootModulePolicyDecision::Deny {
            module_id: module.id.clone(),
            reason: PolicyRejectionReason::ValidationModuleMismatch {
                expected: module.id.clone(),
                actual: validation.module_id().clone(),
            },
        };
    }

    match validation {
        BootModuleValidationResult::Accepted { .. } => BootModulePolicyDecision::Allow {
            module_id: module.id.clone(),
            kind: module.kind,
            capabilities: module.capabilities.clone(),
        },
        BootModuleValidationResult::Rejected { reason, .. } => BootModulePolicyDecision::Deny {
            module_id: module.id.clone(),
            reason: PolicyRejectionReason::Signature(*reason),
        },
    }
}
