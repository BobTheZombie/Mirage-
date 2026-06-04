//! Mock supervised service modules used by the manifest-driven boot skeleton.
//!
//! These services model Mirage's preferred policy boundary: the supervisor admits
//! a signed manifest entry, grants scoped endpoint authority, the service binds
//! an IPC endpoint only after observing that authority, and request handling is
//! expressed as payload-level service logic rather than kernel policy.

use crate::kernel::exec::SpawnTaskRequest;
use crate::kernel::ipc::MessagePayload;
use crate::kernel::process::{ProcessId, ProcessPriority};
use crate::kernel::services::registry::ServiceId as RegistryServiceId;
use crate::kernel::{Kernel, KernelError};
use crate::subkernel::{CapabilityObject, CapabilityRight, CapabilityRights, Credentials};
use crate::supervisor::reset_spawned_service_capabilities;

/// Manifest module id for the echo service.
pub const ECHO_SERVICE_MODULE_ID: &str = "echo-service";

/// Supervisor-owned endpoint name for echo request/reply traffic.
pub const ECHO_IPC_ENDPOINT: &str = "echo.ipc";

/// Mock image path expected from `boot/manifest.mock.toml`.
pub const ECHO_SERVICE_IMAGE: &str = "/boot/services/echo-service.mmod";

/// Capability object string used by the mock boot manifest parser.
pub const IPC_ENDPOINT_CAPABILITY_OBJECT: &str = "IPC_ENDPOINT";

/// Manifest-level capability view consumed after signature and policy validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MockManifestCapability<'a> {
    pub object: &'a str,
    pub endpoint: Option<&'a str>,
    pub rights: &'a [&'a str],
}

/// Manifest-level service launch input consumed by supervisor boot policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MockManifestService<'a> {
    pub module_id: &'a str,
    pub image: &'a str,
    pub restart_always: bool,
    pub capabilities: &'a [MockManifestCapability<'a>],
}

/// Capability token delivered to `echo-service` before endpoint registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EchoEndpointCapability {
    endpoint: RegistryServiceId,
    rights: CapabilityRights,
}

impl EchoEndpointCapability {
    pub const fn new(endpoint: RegistryServiceId, rights: CapabilityRights) -> Self {
        Self { endpoint, rights }
    }

    pub const fn endpoint(self) -> RegistryServiceId {
        self.endpoint
    }

    pub const fn rights(self) -> CapabilityRights {
        self.rights
    }
}

/// Isolated mock echo-service state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EchoService {
    endpoint_capability: Option<EchoEndpointCapability>,
    registered_endpoint: Option<RegistryServiceId>,
}

impl EchoService {
    pub const fn new() -> Self {
        Self {
            endpoint_capability: None,
            registered_endpoint: None,
        }
    }

    /// Accept endpoint authority only when it names `echo.ipc` with send/receive rights.
    pub fn receive_endpoint_capability(
        &mut self,
        capability: EchoEndpointCapability,
    ) -> Result<(), EchoServiceError> {
        if capability.endpoint != RegistryServiceId::EchoIpc {
            return Err(EchoServiceError::WrongEndpoint);
        }
        if !capability.rights.contains(CapabilityRight::Send)
            || !capability.rights.contains(CapabilityRight::Receive)
        {
            return Err(EchoServiceError::InsufficientRights);
        }

        self.endpoint_capability = Some(capability);
        Ok(())
    }

    /// Bind `echo.ipc` only after endpoint capability delivery succeeds.
    pub fn register_endpoint(
        &mut self,
        endpoint: RegistryServiceId,
    ) -> Result<(), EchoServiceError> {
        let capability = self
            .endpoint_capability
            .ok_or(EchoServiceError::EndpointCapabilityMissing)?;
        if endpoint != RegistryServiceId::EchoIpc || capability.endpoint != endpoint {
            return Err(EchoServiceError::WrongEndpoint);
        }

        self.registered_endpoint = Some(endpoint);
        Ok(())
    }

    pub const fn is_registered(&self) -> bool {
        matches!(self.registered_endpoint, Some(RegistryServiceId::EchoIpc))
    }

    /// Echo a request payload back unchanged after the endpoint is registered.
    pub fn echo_payload(
        &self,
        payload: MessagePayload,
    ) -> Result<MessagePayload, EchoServiceError> {
        if !self.is_registered() {
            return Err(EchoServiceError::EndpointNotRegistered);
        }
        Ok(payload)
    }
}

impl Default for EchoService {
    fn default() -> Self {
        Self::new()
    }
}

/// Runtime record for a manifest-launched mock service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MockServiceLaunchReport {
    pub supervisor_pid: ProcessId,
    pub service_pid: ProcessId,
    pub endpoint: RegistryServiceId,
    pub endpoint_capability_id: crate::subkernel::CapabilityId,
    pub service: EchoService,
}

/// Failures from manifest-launched mock service admission and IPC dispatch.
#[derive(Clone, Copy, Debug)]
pub enum MockServiceLaunchError {
    UnknownModule,
    WrongImage,
    RestartPolicyUnsupported,
    EndpointCapabilityMissing,
    EndpointCapabilityInvalid,
    Service(EchoServiceError),
    Kernel(KernelError),
}

impl From<KernelError> for MockServiceLaunchError {
    fn from(error: KernelError) -> Self {
        Self::Kernel(error)
    }
}

impl From<EchoServiceError> for MockServiceLaunchError {
    fn from(error: EchoServiceError) -> Self {
        Self::Service(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoServiceError {
    EndpointCapabilityMissing,
    EndpointNotRegistered,
    WrongEndpoint,
    InsufficientRights,
}

/// Convert a validated manifest capability into the scoped endpoint token that
/// `echo-service` must receive before registering `echo.ipc`.
pub fn echo_endpoint_capability_from_manifest(
    capability: MockManifestCapability<'_>,
) -> Result<EchoEndpointCapability, MockServiceLaunchError> {
    if capability.object != IPC_ENDPOINT_CAPABILITY_OBJECT {
        return Err(MockServiceLaunchError::EndpointCapabilityInvalid);
    }
    if capability.endpoint != Some(ECHO_IPC_ENDPOINT) {
        return Err(MockServiceLaunchError::EndpointCapabilityInvalid);
    }

    let mut rights = CapabilityRights::none();
    let mut saw_send = false;
    let mut saw_receive = false;
    let mut idx = 0usize;
    while idx < capability.rights.len() {
        match capability.rights[idx] {
            "SEND" => {
                rights = rights.with(CapabilityRight::Send);
                saw_send = true;
            }
            "RECEIVE" => {
                rights = rights.with(CapabilityRight::Receive);
                saw_receive = true;
            }
            _ => return Err(MockServiceLaunchError::EndpointCapabilityInvalid),
        }
        idx += 1;
    }

    if !saw_send || !saw_receive {
        return Err(MockServiceLaunchError::EndpointCapabilityMissing);
    }

    Ok(EchoEndpointCapability::new(
        RegistryServiceId::EchoIpc,
        rights,
    ))
}

pub(crate) fn launch_echo_service_from_validated_manifest<
    const NPROC: usize,
    const MSG_DEPTH: usize,
>(
    kernel: &mut Kernel<NPROC, MSG_DEPTH>,
    service: MockManifestService<'_>,
) -> Result<MockServiceLaunchReport, MockServiceLaunchError> {
    if service.module_id != ECHO_SERVICE_MODULE_ID {
        return Err(MockServiceLaunchError::UnknownModule);
    }
    if service.image != ECHO_SERVICE_IMAGE {
        return Err(MockServiceLaunchError::WrongImage);
    }
    if !service.restart_always {
        return Err(MockServiceLaunchError::RestartPolicyUnsupported);
    }

    let mut endpoint_capability = None;
    let mut idx = 0usize;
    while idx < service.capabilities.len() {
        let capability = echo_endpoint_capability_from_manifest(service.capabilities[idx])?;
        endpoint_capability = Some(capability);
        idx += 1;
    }
    let endpoint_capability =
        endpoint_capability.ok_or(MockServiceLaunchError::EndpointCapabilityMissing)?;

    let supervisor_pid = kernel.spawn_initial_process(Credentials::system())?;
    let service_pid = kernel.spawn_task(SpawnTaskRequest {
        parent: Some(supervisor_pid),
        entry_point: 0,
        priority: ProcessPriority::Normal,
        credentials: Credentials::user(),
    })?;

    reset_spawned_service_capabilities(kernel, service_pid)?;
    let endpoint_capability_id = kernel.grant_task_capability(
        service_pid,
        CapabilityObject::IpcEndpoint(ProcessId::new(RegistryServiceId::EchoIpc.raw())),
        endpoint_capability.rights,
    )?;

    let mut echo = EchoService::new();
    echo.receive_endpoint_capability(endpoint_capability)?;
    echo.register_endpoint(RegistryServiceId::EchoIpc)?;
    kernel.register_endpoint(supervisor_pid, RegistryServiceId::EchoIpc, service_pid)?;

    Ok(MockServiceLaunchReport {
        supervisor_pid,
        service_pid,
        endpoint: RegistryServiceId::EchoIpc,
        endpoint_capability_id,
        service: echo,
    })
}

pub(crate) fn dispatch_echo_request<const NPROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<NPROC, MSG_DEPTH>,
    report: &MockServiceLaunchReport,
    caller: ProcessId,
    payload: MessagePayload,
) -> Result<MessagePayload, MockServiceLaunchError> {
    kernel.send_service_message(caller, RegistryServiceId::EchoIpc, payload)?;
    let request = kernel.receive_message(report.service_pid)?;
    let response = report.service.echo_payload(request.payload)?;
    kernel.grant_task_capability(
        report.service_pid,
        CapabilityObject::IpcEndpoint(request.sender),
        CapabilityRights::ipc_endpoint(),
    )?;
    kernel.send_message(report.service_pid, request.sender, response)?;
    kernel
        .receive_message(caller)
        .map(|message| message.payload)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subkernel::SecurityClass;

    #[test]
    fn echo_service_requires_endpoint_capability_before_registration() {
        let mut service = EchoService::new();
        assert_eq!(
            service.register_endpoint(RegistryServiceId::EchoIpc),
            Err(EchoServiceError::EndpointCapabilityMissing)
        );

        service
            .receive_endpoint_capability(EchoEndpointCapability::new(
                RegistryServiceId::EchoIpc,
                CapabilityRights::ipc_endpoint(),
            ))
            .unwrap();
        service
            .register_endpoint(RegistryServiceId::EchoIpc)
            .unwrap();
        assert!(service.is_registered());
    }

    #[test]
    fn echo_service_returns_request_payload_unchanged() {
        let mut service = EchoService::new();
        service
            .receive_endpoint_capability(EchoEndpointCapability::new(
                RegistryServiceId::EchoIpc,
                CapabilityRights::ipc_endpoint(),
            ))
            .unwrap();
        service
            .register_endpoint(RegistryServiceId::EchoIpc)
            .unwrap();

        let payload = MessagePayload::from_slice(SecurityClass::Internal, b"hello mirage");
        assert_eq!(service.echo_payload(payload), Ok(payload));
    }

    #[test]
    fn echo_service_launches_from_validated_manifest_and_registers_echo_ipc() {
        let manifest =
            mirage_boot::parse_manifest_toml(include_str!("../../boot/manifest.mock.toml"))
                .expect("manifest parses");
        let plans = mirage_boot::build_service_launch_plan(&manifest).expect("manifest validates");
        let plan = plans
            .iter()
            .find(|plan| plan.module_id.as_str() == ECHO_SERVICE_MODULE_ID)
            .expect("echo-service is described by the manifest");

        let rights: Vec<&str> = plan.capabilities[0]
            .rights
            .iter()
            .map(String::as_str)
            .collect();
        let capability = MockManifestCapability {
            object: plan.capabilities[0].object.as_str(),
            endpoint: plan.capabilities[0].endpoint.as_deref(),
            rights: rights.as_slice(),
        };
        let capabilities = [capability];
        let service = MockManifestService {
            module_id: plan.module_id.as_str(),
            image: plan.image.as_str(),
            restart_always: plan.restart == mirage_boot::RestartPolicy::Always,
            capabilities: &capabilities,
        };

        let mut kernel = Kernel::<16, 4>::new();
        kernel.bootstrap();
        let supervisor = crate::supervisor::Supervisor::new();
        let report = supervisor
            .launch_mock_manifest_service(&mut kernel, service)
            .expect("supervisor launches manifest-admitted echo service");

        assert!(report.service.is_registered());
        assert_eq!(report.endpoint, RegistryServiceId::EchoIpc);
        assert_eq!(
            kernel.service_owner(RegistryServiceId::EchoIpc),
            Some(report.service_pid)
        );
    }

    #[test]
    fn echo_service_echoes_payload_through_manifest_supervisor_path() {
        let manifest =
            mirage_boot::parse_manifest_toml(include_str!("../../boot/manifest.mock.toml"))
                .expect("manifest parses");
        let plans = mirage_boot::build_service_launch_plan(&manifest).expect("manifest validates");
        let plan = plans
            .iter()
            .find(|plan| plan.module_id.as_str() == ECHO_SERVICE_MODULE_ID)
            .expect("echo-service is described by the manifest");

        let rights: Vec<&str> = plan.capabilities[0]
            .rights
            .iter()
            .map(String::as_str)
            .collect();
        let capability = MockManifestCapability {
            object: plan.capabilities[0].object.as_str(),
            endpoint: plan.capabilities[0].endpoint.as_deref(),
            rights: rights.as_slice(),
        };
        let capabilities = [capability];
        let service = MockManifestService {
            module_id: plan.module_id.as_str(),
            image: plan.image.as_str(),
            restart_always: plan.restart == mirage_boot::RestartPolicy::Always,
            capabilities: &capabilities,
        };

        let mut kernel = Kernel::<16, 8>::new();
        kernel.bootstrap();
        let supervisor = crate::supervisor::Supervisor::new();
        let report = supervisor
            .launch_mock_manifest_service(&mut kernel, service)
            .expect("supervisor launches manifest-admitted echo service");
        let caller = kernel
            .spawn_initial_process(Credentials::system())
            .expect("caller exists");
        let payload = MessagePayload::from_slice(SecurityClass::Internal, b"echo over echo.ipc");

        let response = supervisor
            .dispatch_echo_request(&mut kernel, &report, caller, payload)
            .expect("echo request/reply succeeds");

        assert_eq!(response, payload);
    }
}
