//! Boot manifest policy data for GNU/Mirage.
//!
//! This crate intentionally models supervisor-owned boot policy data outside the
//! mechanism-only kernel path. Parsing, mock signature verification, policy
//! admission, and launch planning live in separate modules so the supervisor can
//! make decisions while the kernel remains responsible only for enforcing the
//! resulting low-level primitives.

#![forbid(unsafe_code)]

pub mod launch;
pub mod manifest;
pub mod parser;
pub mod policy;
pub mod signature;

pub use launch::{build_service_launch_plan, LaunchPlanError, ServiceLaunchPlan};
pub use manifest::{
    BootModule, BootModuleCapabilityRequest, BootModuleId, BootModuleKind, BootModuleManifest,
    BootModuleSignature, RestartPolicy,
};
pub use parser::{parse_manifest_toml, BootManifestParseError};
pub use policy::{decide_module_policy, BootModulePolicyDecision, PolicyRejectionReason};
pub use signature::{verify_mock_signature, BootModuleValidationResult, ValidationFailureReason};

#[cfg(test)]
mod tests {
    use super::*;

    const MOCK_MANIFEST: &str = include_str!("../../../boot/manifest.mock.toml");

    #[test]
    fn parses_echo_service_mock_manifest() {
        let manifest = parse_manifest_toml(MOCK_MANIFEST).expect("mock manifest parses");
        let module = manifest
            .module(&BootModuleId("echo-service".to_string()))
            .expect("echo-service module exists");

        assert_eq!(module.kind, BootModuleKind::Service);
        assert_eq!(module.signature.value, "mock-valid");
        assert_eq!(module.restart, RestartPolicy::Always);
        assert_eq!(module.capabilities[0].object, "IPC_ENDPOINT");
        assert_eq!(module.capabilities[0].endpoint.as_deref(), Some("echo.ipc"));
        assert_eq!(module.capabilities[0].rights, ["SEND", "RECEIVE"]);
    }

    #[test]
    fn plans_valid_service_launch_without_kernel_policy() {
        let manifest = parse_manifest_toml(MOCK_MANIFEST).expect("mock manifest parses");
        let plans = build_service_launch_plan(&manifest).expect("valid service plan");

        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].module_id.as_str(), "echo-service");
        assert_eq!(plans[0].restart, RestartPolicy::Always);
        assert_eq!(plans[0].capabilities[0].object, "IPC_ENDPOINT");
    }
}
