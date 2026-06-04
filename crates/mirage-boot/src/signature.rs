use crate::manifest::{BootModule, BootModuleId};

/// Result of validating manifest signature metadata for a boot module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BootModuleValidationResult {
    Accepted {
        module_id: BootModuleId,
    },
    Rejected {
        module_id: BootModuleId,
        reason: ValidationFailureReason,
    },
}

impl BootModuleValidationResult {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted { .. })
    }

    pub fn module_id(&self) -> &BootModuleId {
        match self {
            Self::Accepted { module_id } | Self::Rejected { module_id, .. } => module_id,
        }
    }
}

/// Mock validation failures used until real cryptographic verification exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationFailureReason {
    InvalidMockSignature,
}

/// Verify a module with the explicit development-only `mock-valid` signature.
pub fn verify_mock_signature(module: &BootModule) -> BootModuleValidationResult {
    if module.signature.is_mock_valid() {
        BootModuleValidationResult::Accepted {
            module_id: module.id.clone(),
        }
    } else {
        BootModuleValidationResult::Rejected {
            module_id: module.id.clone(),
            reason: ValidationFailureReason::InvalidMockSignature,
        }
    }
}
