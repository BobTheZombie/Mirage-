#![no_std]
#![forbid(unsafe_code)]

//! Supervisor-facing platform policy orchestration.
//!
//! This crate is intentionally above mechanism crates. It prepares service
//! launch and handoff records from supervisor-issued capabilities, but it does
//! not perform raw hardware access.

use mirage_cap::{CapabilityObject, CapabilityRights, CapabilitySet};
use mirage_ipc::EndpointId;

/// Platform services that may be launched under supervisor control.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum PlatformServiceKind {
    AmdChipset,
    AmdIommu,
    AmdTelemetry,
}

/// Restart behavior requested from the Mirage supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartPolicy {
    RestartOnCrash,
    ManualRecovery,
}

/// Generic supervised-driver launch request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceLaunchRequest {
    pub kind: PlatformServiceKind,
    pub endpoint: EndpointId,
    pub restart_policy: RestartPolicy,
}

impl ServiceLaunchRequest {
    pub const fn new(
        kind: PlatformServiceKind,
        endpoint: EndpointId,
        restart_policy: RestartPolicy,
    ) -> Self {
        Self {
            kind,
            endpoint,
            restart_policy,
        }
    }
}

/// Capability bundle handed to a supervised platform driver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupervisorHandoff {
    pub launch: ServiceLaunchRequest,
    pub capabilities: CapabilitySet,
}

impl SupervisorHandoff {
    pub const fn new(launch: ServiceLaunchRequest, capabilities: CapabilitySet) -> Self {
        Self {
            launch,
            capabilities,
        }
    }

    pub fn validate_endpoint_capability(&self) -> Result<(), mirage_cap::CapabilityError> {
        self.capabilities.check(
            CapabilityObject::IpcEndpoint(self.launch.endpoint.get()),
            CapabilityRights::ipc(),
        )
    }
}

/// Policy planner for AMD platform services.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AmdPlatformPolicy;

impl AmdPlatformPolicy {
    pub const fn chipset_service(endpoint: EndpointId) -> ServiceLaunchRequest {
        ServiceLaunchRequest::new(
            PlatformServiceKind::AmdChipset,
            endpoint,
            RestartPolicy::RestartOnCrash,
        )
    }

    pub const fn iommu_service(endpoint: EndpointId) -> ServiceLaunchRequest {
        ServiceLaunchRequest::new(
            PlatformServiceKind::AmdIommu,
            endpoint,
            RestartPolicy::RestartOnCrash,
        )
    }

    pub const fn telemetry_service(endpoint: EndpointId) -> ServiceLaunchRequest {
        ServiceLaunchRequest::new(
            PlatformServiceKind::AmdTelemetry,
            endpoint,
            RestartPolicy::ManualRecovery,
        )
    }
}
