//! GNU/Mirage supervisor policy.
//!
//! This module is the policy/recovery/security broker outside the mechanism-only
//! kernel path: boot ordering, lifecycle state, signed boot module validation,
//! daemon registration and supervised driver service launch. It deliberately avoids
//! allocation: service descriptors live in a fixed-size manifest and startup
//! progress is captured in a same-capacity report.

use crate::kernel::device::DeviceId;
use crate::kernel::exec::SpawnTaskRequest;
use crate::kernel::process::{
    ExecServiceDaemon, ExecSignatureMetadata, ProcessId, ProcessPriority,
};
use crate::kernel::services::registry::ServiceId as RegistryServiceId;
use crate::kernel::{Kernel, KernelError, ProcessExitReport};
use mirage_platform::{DeviceCandidateRole, DeviceDiscoveryEvent, PlatformInfo, PlatformService};
pub mod i8042;
pub mod input;
pub mod mock_service;

pub mod renoir_mtss;
pub mod renoir_xhci;

pub mod pid1;
pub mod usb;

use crate::subkernel::{
    CapabilityId, CapabilityObject, CapabilityRights, CapabilitySet, Credentials, IsolationError,
    IsolationLevel, SecurityLabel,
};

/// Number of services in the built-in signed boot module manifest.
pub const MAX_STARTUP_SERVICES: usize = 9;

/// Maximum number of dependencies a startup service can declare.
pub const MAX_SERVICE_DEPENDENCIES: usize = 2;

/// Well-known GNU/Mirage startup services.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceId {
    L2Subkernel,
    Storaged,
    Usbd,
    Nvmed,
    Ahcid,
    Displayd,
    AmdgpuDisplayd,
    Networkd,
    Inputd,
}

/// Boot-service lifecycle state for a manifest-launched service supervised by the policy broker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupState {
    /// Runtime record exists but no policy or launch work has started yet.
    Created,
    /// The supervisor is validating manifest policy and signatures.
    Validating,
    /// The supervisor is asking the kernel to launch the service task.
    Launching,
    /// The service task is live and owns its supervised authority.
    Running,
    /// The previously running service exited and recovery handling observed the crash.
    Crashed,
    /// The supervisor is attempting to relaunch the service after a crash.
    Restarting,
    /// The supervisor intentionally left the service offline.
    Stopped,
    /// Supervisor policy denied the service launch or relaunch.
    Denied,
}

/// Fixed-size service dependency list.
pub type ServiceDependencies = [Option<ServiceId>; MAX_SERVICE_DEPENDENCIES];

/// Static service launch metadata.
#[derive(Clone, Copy, Debug)]
pub struct ServiceDescriptor {
    pub id: ServiceId,
    pub name: &'static str,
    pub entry_point: u64,
    pub priority: ProcessPriority,
    pub credentials: Credentials,
    pub dependencies: ServiceDependencies,
    pub service_daemon: Option<ExecServiceDaemon>,
    pub signature: Option<ExecSignatureMetadata>,
}

impl ServiceDescriptor {
    pub const fn new(
        id: ServiceId,
        name: &'static str,
        entry_point: u64,
        priority: ProcessPriority,
        credentials: Credentials,
        dependencies: ServiceDependencies,
        service_daemon: Option<ExecServiceDaemon>,
        signature: Option<ExecSignatureMetadata>,
    ) -> Self {
        Self {
            id,
            name,
            entry_point,
            priority,
            credentials,
            dependencies,
            service_daemon,
            signature,
        }
    }
}

/// Fixed-capacity no-alloc service manifest.
#[derive(Clone, Copy, Debug)]
pub struct ServiceManifest<const CAP: usize> {
    descriptors: [Option<ServiceDescriptor>; CAP],
    len: usize,
}

impl<const CAP: usize> ServiceManifest<CAP> {
    pub const fn new(descriptors: [Option<ServiceDescriptor>; CAP], len: usize) -> Self {
        Self { descriptors, len }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn capacity(&self) -> usize {
        CAP
    }

    pub fn descriptor(&self, index: usize) -> Option<ServiceDescriptor> {
        if index < self.len {
            self.descriptors[index]
        } else {
            None
        }
    }
}

/// Maximum number of supervisor-managed capabilities recorded per service.
pub const MAX_SERVICE_CAPABILITIES: usize = 9;

/// Maximum number of supervisor-managed device claims recorded per service.
pub const MAX_SERVICE_DEVICE_CLAIMS: usize = 4;

/// Maximum number of supervisor-managed IPC endpoints recorded per service.
pub const MAX_SERVICE_ENDPOINTS: usize = 2;

/// Policy the supervisor applies after a managed service exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartPolicy {
    Never,
    Always,
}

/// Static policy inputs used to mint a hardware capability bundle for a driver service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HardwareCapabilityPolicy {
    pub pci_device: u64,
    pub mmio_base: u64,
    pub mmio_length: u64,
    pub dma_base: Option<u64>,
    pub dma_length: Option<u64>,
    pub irq_line: Option<u16>,
    pub vram_base: Option<u64>,
    pub vram_length: Option<u64>,
    pub framebuffer_base: Option<u64>,
    pub framebuffer_length: Option<u64>,
}

impl HardwareCapabilityPolicy {
    pub const fn new(pci_device: u64, mmio_base: u64, mmio_length: u64) -> Self {
        Self {
            pci_device,
            mmio_base,
            mmio_length,
            dma_base: None,
            dma_length: None,
            irq_line: None,
            vram_base: None,
            vram_length: None,
            framebuffer_base: None,
            framebuffer_length: None,
        }
    }

    pub const fn with_dma(mut self, base: u64, length: u64) -> Self {
        self.dma_base = Some(base);
        self.dma_length = Some(length);
        self
    }

    pub const fn with_irq(mut self, line: u16) -> Self {
        self.irq_line = Some(line);
        self
    }

    pub const fn with_vram(mut self, base: u64, length: u64) -> Self {
        self.vram_base = Some(base);
        self.vram_length = Some(length);
        self
    }

    pub const fn with_framebuffer(mut self, base: u64, length: u64) -> Self {
        self.framebuffer_base = Some(base);
        self.framebuffer_length = Some(length);
        self
    }
}

/// Capability assignment owned by a supervisor runtime record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceCapabilityAssignment {
    pub object: CapabilityObject,
    pub rights: CapabilityRights,
    pub id: Option<CapabilityId>,
}

impl ServiceCapabilityAssignment {
    pub const fn new(object: CapabilityObject, rights: CapabilityRights) -> Self {
        Self {
            object,
            rights,
            id: None,
        }
    }

    const fn assigned(self, id: CapabilityId) -> Self {
        Self {
            object: self.object,
            rights: self.rights,
            id: Some(id),
        }
    }

    const fn unassigned(self) -> Self {
        Self {
            object: self.object,
            rights: self.rights,
            id: None,
        }
    }
}

/// Device claim that must be released before restart and restored afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceDeviceClaim {
    pub service: RegistryServiceId,
    pub device: DeviceId,
}

/// IPC endpoint registration that must be restored after restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceEndpointClaim {
    pub service: RegistryServiceId,
}

/// Supervisor-approved MTSS priority-change request.
///
/// This is intentionally only a policy envelope today: MTSS owns the eventual
/// priority transition and the kernel/backend owns any hardware-visible
/// scheduling effect. The supervisor may validate that a service is allowed to
/// ask for a new priority, but it must not rewrite MTSS queues itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupervisorMtssPriorityRequest {
    pub task: ProcessId,
    pub requested_priority: ProcessPriority,
}

impl SupervisorMtssPriorityRequest {
    pub const fn new(task: ProcessId, requested_priority: ProcessPriority) -> Self {
        Self {
            task,
            requested_priority,
        }
    }
}

/// Clean supervisor-to-MTSS policy boundary.
///
/// Supervisor lifecycle code may approve policy requests, but it must enter
/// multitasking through this boundary only. The intended flow is:
///
/// ```text
/// supervisor policy request
///   -> MTSS state transition
///   -> kernel/backend execution later
/// ```
///
/// Keep recovery policy here in the supervisor; keep runnable-state, queue, and
/// scheduling mechanics behind MTSS/kernel integration. In particular,
/// supervisor code must not inspect or mutate MTSS run queues directly.

/// Supervisor-owned policy gate for replacing a process image with exec.
///
/// The kernel prepares and validates the executable image, but the decision to
/// authorize the image replacement remains supervisor policy.
pub trait SupervisorExecPolicy {
    fn supervisor_authorize_exec(
        &self,
        request: &crate::kernel::process::ExecRequest,
    ) -> Result<(), KernelError>;
}

pub trait SupervisorMtssBoundary {
    fn mtss_spawn_task(&mut self, request: SpawnTaskRequest) -> Result<ProcessId, KernelError>;
    fn mtss_contain_task(&mut self, task: ProcessId) -> Result<(), KernelError>;
    fn mtss_terminate_task(&mut self, task: ProcessId) -> Result<(), KernelError>;
    fn mtss_reap_task(&mut self, task: ProcessId) -> Result<(), KernelError>;
    fn mtss_request_priority(
        &mut self,
        request: SupervisorMtssPriorityRequest,
    ) -> Result<(), KernelError>;
}

impl<const NPROC: usize, const MSG_DEPTH: usize> SupervisorMtssBoundary
    for Kernel<NPROC, MSG_DEPTH>
{
    fn mtss_spawn_task(&mut self, request: SpawnTaskRequest) -> Result<ProcessId, KernelError> {
        self.spawn_task(request)
    }

    fn mtss_contain_task(&mut self, _task: ProcessId) -> Result<(), KernelError> {
        // TODO(mtss): route to the real MTSS containment transition once the
        // kernel exposes it. The supervisor still calls this boundary so
        // recovery policy never reaches into MTSS run queues.
        Ok(())
    }

    fn mtss_terminate_task(&mut self, task: ProcessId) -> Result<(), KernelError> {
        self.terminate_process(task);
        Ok(())
    }

    fn mtss_reap_task(&mut self, _task: ProcessId) -> Result<(), KernelError> {
        // TODO(mtss): route to MTSS reaping when the kernel/backend exposes a
        // supervisor-facing reap primitive distinct from POSIX wait handling.
        Ok(())
    }

    fn mtss_request_priority(
        &mut self,
        _request: SupervisorMtssPriorityRequest,
    ) -> Result<(), KernelError> {
        // TODO(mtss): validate and forward priority changes once MTSS priority
        // transitions are part of the kernel integration surface.
        Err(KernelError::InvalidArgument)
    }
}

/// Result of supervisor crash/exit handling for one kernel exit report.
#[derive(Clone, Copy, Debug)]
pub enum ServiceRecoveryState {
    NotSupervised,
    Restarted(ProcessId),
    Stopped,
    Failed(KernelError),
}

/// Per-service startup and recovery state recorded by the supervisor broker.
#[derive(Clone, Copy, Debug)]
pub struct ServiceRuntime {
    pub descriptor: ServiceDescriptor,
    pub state: StartupState,
    pub pid: Option<ProcessId>,
    pub assigned_capabilities: [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES],
    pub claimed_devices: [Option<ServiceDeviceClaim>; MAX_SERVICE_DEVICE_CLAIMS],
    pub claimed_endpoints: [Option<ServiceEndpointClaim>; MAX_SERVICE_ENDPOINTS],
    pub restart_policy: RestartPolicy,
    pub last_exit_status: Option<crate::kernel::process::ExitStatus>,
    pub failure: Option<KernelError>,
    pub restart_generation: u64,
}

impl ServiceRuntime {
    pub const fn pending(descriptor: ServiceDescriptor) -> Self {
        Self {
            descriptor,
            state: StartupState::Created,
            pid: None,
            assigned_capabilities: [None; MAX_SERVICE_CAPABILITIES],
            claimed_devices: [None; MAX_SERVICE_DEVICE_CLAIMS],
            claimed_endpoints: [None; MAX_SERVICE_ENDPOINTS],
            restart_policy: RestartPolicy::Always,
            last_exit_status: None,
            failure: None,
            restart_generation: 0,
        }
    }

    fn with_default_capability_specs(mut self) -> Self {
        self.assigned_capabilities = default_capability_specs(self.descriptor);
        self
    }

    fn record_device_claim(&mut self, service: RegistryServiceId, device: DeviceId) -> bool {
        let mut idx = 0usize;
        while idx < MAX_SERVICE_DEVICE_CLAIMS {
            if self.claimed_devices[idx] == Some(ServiceDeviceClaim { service, device }) {
                return true;
            }
            if self.claimed_devices[idx].is_none() {
                self.claimed_devices[idx] = Some(ServiceDeviceClaim { service, device });
                return true;
            }
            idx += 1;
        }
        false
    }

    fn record_endpoint_claim(&mut self, service: RegistryServiceId) -> bool {
        let mut idx = 0usize;
        while idx < MAX_SERVICE_ENDPOINTS {
            if self.claimed_endpoints[idx] == Some(ServiceEndpointClaim { service }) {
                return true;
            }
            if self.claimed_endpoints[idx].is_none() {
                self.claimed_endpoints[idx] = Some(ServiceEndpointClaim { service });
                return true;
            }
            idx += 1;
        }
        false
    }
}

/// Fixed-capacity startup report produced by the supervisor broker.
#[derive(Clone, Copy, Debug)]
pub struct ServiceStartupReport<const CAP: usize> {
    records: [Option<ServiceRuntime>; CAP],
    len: usize,
}

impl<const CAP: usize> ServiceStartupReport<CAP> {
    pub const fn new() -> Self {
        Self {
            records: [None; CAP],
            len: 0,
        }
    }

    pub fn from_manifest(manifest: &ServiceManifest<CAP>) -> Self {
        let mut report = Self::new();
        let mut idx = 0usize;
        while idx < manifest.len() {
            if let Some(descriptor) = manifest.descriptor(idx) {
                report.records[idx] =
                    Some(ServiceRuntime::pending(descriptor).with_default_capability_specs());
                report.len += 1;
            }
            idx += 1;
        }
        report
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub fn record(&self, index: usize) -> Option<ServiceRuntime> {
        if index < self.len {
            self.records[index]
        } else {
            None
        }
    }

    pub fn state(&self, service: ServiceId) -> Option<StartupState> {
        self.find(service).map(|record| record.state)
    }

    pub fn pid(&self, service: ServiceId) -> Option<ProcessId> {
        self.find(service).and_then(|record| record.pid)
    }

    pub fn generation(&self, service: ServiceId) -> Option<u64> {
        self.find(service).map(|record| record.restart_generation)
    }

    pub fn last_exit_status(
        &self,
        service: ServiceId,
    ) -> Option<crate::kernel::process::ExitStatus> {
        self.find(service)
            .and_then(|record| record.last_exit_status)
    }

    pub fn record_device_claim(
        &mut self,
        service: ServiceId,
        registry_service: RegistryServiceId,
        device: DeviceId,
    ) -> bool {
        if let Some(index) = self.find_index(service) {
            if let Some(mut record) = self.records[index] {
                let recorded = record.record_device_claim(registry_service, device);
                self.records[index] = Some(record);
                return recorded;
            }
        }
        false
    }

    pub fn record_endpoint_claim(
        &mut self,
        service: ServiceId,
        registry_service: RegistryServiceId,
    ) -> bool {
        if let Some(index) = self.find_index(service) {
            if let Some(mut record) = self.records[index] {
                let recorded = record.record_endpoint_claim(registry_service);
                self.records[index] = Some(record);
                return recorded;
            }
        }
        false
    }

    pub fn handle_process_exit<const NPROC: usize, const MSG_DEPTH: usize>(
        &mut self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        report: ProcessExitReport,
    ) -> ServiceRecoveryState {
        let Some(index) = self.find_pid_index(report.pid) else {
            return ServiceRecoveryState::NotSupervised;
        };

        let _ = kernel.mtss_contain_task(report.pid);
        self.release_record_claims(kernel, index, report.pid);
        self.revoke_record_hardware_resources(kernel, index);
        kernel.revoke_task(report.pid);
        kernel.revoke_service_owner(report.pid);
        let _ = kernel.mtss_reap_task(report.pid);

        if let Some(mut record) = self.records[index] {
            record.pid = None;
            record.state = StartupState::Crashed;
            record.last_exit_status = Some(report.status);
            record.failure = None;
            self.records[index] = Some(record);
        }

        let record = self.records[index].unwrap();
        if record.restart_policy == RestartPolicy::Never {
            let _ = kernel.mtss_terminate_task(report.pid);
            self.set_stopped(index);
            return ServiceRecoveryState::Stopped;
        }

        self.set_restarting(index);

        match self.respawn_record(kernel, index) {
            Ok(pid) => ServiceRecoveryState::Restarted(pid),
            Err(error) => {
                self.set_denied(index, error);
                ServiceRecoveryState::Failed(error)
            }
        }
    }

    pub fn all_running(&self) -> bool {
        let mut idx = 0usize;
        while idx < self.len {
            if let Some(record) = self.records[idx] {
                if record.state != StartupState::Running {
                    return false;
                }
            }
            idx += 1;
        }
        true
    }

    fn set_validating(&mut self, index: usize) {
        if let Some(mut record) = self.records[index] {
            record.state = StartupState::Validating;
            record.failure = None;
            self.records[index] = Some(record);
        }
    }

    fn set_launching(&mut self, index: usize) {
        if let Some(mut record) = self.records[index] {
            record.state = StartupState::Launching;
            record.failure = None;
            self.records[index] = Some(record);
        }
    }

    fn set_running(&mut self, index: usize, pid: ProcessId) {
        if let Some(mut record) = self.records[index] {
            record.state = StartupState::Running;
            record.pid = Some(pid);
            record.failure = None;
            self.records[index] = Some(record);
        }
    }

    fn set_denied(&mut self, index: usize, error: KernelError) {
        if let Some(mut record) = self.records[index] {
            record.state = StartupState::Denied;
            record.pid = None;
            record.failure = Some(error);
            self.records[index] = Some(record);
        }
    }

    fn set_restarting(&mut self, index: usize) {
        if let Some(mut record) = self.records[index] {
            record.state = StartupState::Restarting;
            record.failure = None;
            self.records[index] = Some(record);
        }
    }

    fn set_stopped(&mut self, index: usize) {
        if let Some(mut record) = self.records[index] {
            record.state = StartupState::Stopped;
            record.pid = None;
            self.records[index] = Some(record);
        }
    }

    fn dependency_state(&self, service: ServiceId) -> Option<StartupState> {
        self.state(service)
    }

    fn dependency_pid(&self, service: ServiceId) -> Option<ProcessId> {
        self.pid(service)
    }

    fn find_index(&self, service: ServiceId) -> Option<usize> {
        let mut idx = 0usize;
        while idx < self.len {
            if let Some(record) = self.records[idx] {
                if record.descriptor.id == service {
                    return Some(idx);
                }
            }
            idx += 1;
        }
        None
    }

    fn find_pid_index(&self, pid: ProcessId) -> Option<usize> {
        let mut idx = 0usize;
        while idx < self.len {
            if self.records[idx]
                .and_then(|record| record.pid)
                .map(|owner| owner == pid)
                .unwrap_or(false)
            {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    fn parent_for(&self, descriptor: ServiceDescriptor) -> Option<ProcessId> {
        let mut dep_idx = 0usize;
        while dep_idx < MAX_SERVICE_DEPENDENCIES {
            if let Some(dependency) = descriptor.dependencies[dep_idx] {
                if let Some(pid) = self.dependency_pid(dependency) {
                    return Some(pid);
                }
            }
            dep_idx += 1;
        }
        None
    }

    fn revoke_record_hardware_resources<const NPROC: usize, const MSG_DEPTH: usize>(
        &mut self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        index: usize,
    ) {
        if let Some(mut record) = self.records[index] {
            let mut cap_idx = 0usize;
            while cap_idx < MAX_SERVICE_CAPABILITIES {
                if let Some(capability) = record.assigned_capabilities[cap_idx] {
                    if Self::is_crash_reclaimed_capability(capability.object) {
                        if let Some(id) = capability.id {
                            let _ = kernel.revoke_task_capability(id);
                        }
                        record.assigned_capabilities[cap_idx] = Some(capability.unassigned());
                    }
                }
                cap_idx += 1;
            }
            self.records[index] = Some(record);
        }
    }

    fn is_crash_reclaimed_capability(object: CapabilityObject) -> bool {
        matches!(
            object,
            CapabilityObject::MmioRegion { .. }
                | CapabilityObject::DmaRegion { .. }
                | CapabilityObject::IrqLine(_)
                | CapabilityObject::VramRegion { .. }
                | CapabilityObject::Framebuffer { .. }
        )
    }

    fn release_record_claims<const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        index: usize,
        owner: ProcessId,
    ) {
        if let Some(record) = self.records[index] {
            let mut device_idx = 0usize;
            while device_idx < MAX_SERVICE_DEVICE_CLAIMS {
                if let Some(claim) = record.claimed_devices[device_idx] {
                    let _ = kernel.release_service_device(owner, claim.service, claim.device);
                }
                device_idx += 1;
            }
        }
    }

    fn regrant_record_capabilities<const NPROC: usize, const MSG_DEPTH: usize>(
        &mut self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        index: usize,
        pid: ProcessId,
    ) -> Result<(), KernelError> {
        if let Some(mut record) = self.records[index] {
            let mut cap_idx = 0usize;
            while cap_idx < MAX_SERVICE_CAPABILITIES {
                if let Some(capability) = record.assigned_capabilities[cap_idx] {
                    let id =
                        kernel.grant_task_capability(pid, capability.object, capability.rights)?;
                    record.assigned_capabilities[cap_idx] = Some(capability.assigned(id));
                }
                cap_idx += 1;
            }
            self.records[index] = Some(record);
        }
        Ok(())
    }

    fn restore_record_claims<const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        index: usize,
        pid: ProcessId,
        authorizer: Option<ProcessId>,
    ) -> Result<(), KernelError> {
        if let Some(record) = self.records[index] {
            let mut endpoint_idx = 0usize;
            while endpoint_idx < MAX_SERVICE_ENDPOINTS {
                if let Some(endpoint) = record.claimed_endpoints[endpoint_idx] {
                    if let Some(authorizer) = authorizer {
                        kernel.register_endpoint(authorizer, endpoint.service, pid)?;
                    }
                }
                endpoint_idx += 1;
            }

            let mut device_idx = 0usize;
            while device_idx < MAX_SERVICE_DEVICE_CLAIMS {
                if let Some(claim) = record.claimed_devices[device_idx] {
                    kernel.claim_service_device(pid, claim.service, claim.device)?;
                }
                device_idx += 1;
            }
        }
        Ok(())
    }

    fn respawn_record<const NPROC: usize, const MSG_DEPTH: usize>(
        &mut self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        index: usize,
    ) -> Result<ProcessId, KernelError> {
        let record = self.records[index].ok_or(KernelError::InvalidArgument)?;
        if !service_manifest_signature_valid(record.descriptor) {
            return Err(KernelError::SecurityViolation(
                IsolationError::PolicyViolation,
            ));
        }
        match dependencies_ready(record.descriptor, self) {
            DependencyStatus::Ready(_) => {}
            DependencyStatus::Waiting | DependencyStatus::Failed => {
                return Err(KernelError::InvalidArgument)
            }
        }
        let parent = self.parent_for(record.descriptor);
        let pid = kernel.mtss_spawn_task(SpawnTaskRequest {
            parent,
            entry_point: record.descriptor.entry_point,
            priority: record.descriptor.priority,
            credentials: record.descriptor.credentials,
        })?;

        reset_spawned_service_capabilities(kernel, pid)?;
        self.regrant_record_capabilities(kernel, index, pid)?;
        self.restore_record_claims(kernel, index, pid, parent)?;

        if let Some(mut updated) = self.records[index] {
            updated.pid = Some(pid);
            updated.state = StartupState::Running;
            updated.failure = None;
            updated.restart_generation = updated.restart_generation.wrapping_add(1);
            self.records[index] = Some(updated);
        }
        Ok(pid)
    }

    fn find(&self, service: ServiceId) -> Option<ServiceRuntime> {
        let mut idx = 0usize;
        while idx < self.len {
            if let Some(record) = self.records[idx] {
                if record.descriptor.id == service {
                    return Some(record);
                }
            }
            idx += 1;
        }
        None
    }
}

/// System L2 service descriptor. It is the only service without a parent and is
/// therefore launched through the initial process path.
pub const L2_SUBKERNEL_SERVICE: ServiceDescriptor = ServiceDescriptor::new(
    ServiceId::L2Subkernel,
    "l2-subkernel",
    0,
    ProcessPriority::Critical,
    Credentials::system(),
    [None, None],
    None,
    Some(ExecSignatureMetadata::new(
        "mirage-l2-root",
        0x4c325f5355424b45,
    )),
);

/// Display daemon descriptor; device-facing and dependent on L2 authorization.
pub const DISPLAYD_SERVICE: ServiceDescriptor = ServiceDescriptor::new(
    ServiceId::Displayd,
    "displayd",
    0,
    ProcessPriority::High,
    Credentials::new(
        SecurityLabel::internal(),
        CapabilitySet::ipc(),
        IsolationLevel::Process,
    ),
    [Some(ServiceId::L2Subkernel), None],
    Some(ExecServiceDaemon::Display),
    Some(ExecSignatureMetadata::new(
        "mirage-service-root",
        0x444953504c415944,
    )),
);

/// Storage service descriptor; brokered endpoint for block-oriented storage clients.
pub const STORAGED_SERVICE: ServiceDescriptor = ServiceDescriptor::new(
    ServiceId::Storaged,
    "storaged",
    0,
    ProcessPriority::High,
    Credentials::new(
        SecurityLabel::confidential(),
        CapabilitySet::ipc(),
        IsolationLevel::Process,
    ),
    [Some(ServiceId::L2Subkernel), None],
    Some(ExecServiceDaemon::L2Driver),
    Some(ExecSignatureMetadata::new(
        "mirage-driver-root",
        0x53544f5241474544,
    )),
);

/// USB daemon descriptor; receives scoped controller, IRQ, and DMA capabilities from L2.
pub const USBD_SERVICE: ServiceDescriptor = ServiceDescriptor::new(
    ServiceId::Usbd,
    "usbd",
    0,
    ProcessPriority::High,
    Credentials::new(
        SecurityLabel::internal(),
        CapabilitySet::ipc(),
        IsolationLevel::Process,
    ),
    [Some(ServiceId::L2Subkernel), None],
    Some(ExecServiceDaemon::L2Driver),
    Some(ExecSignatureMetadata::new(
        "mirage-driver-root",
        0x555342445f444d4e,
    )),
);

/// NVMe daemon descriptor; receives only its scoped PCI/MMIO/DMA/IRQ capabilities from L2.
pub const NVMED_SERVICE: ServiceDescriptor = ServiceDescriptor::new(
    ServiceId::Nvmed,
    "nvmed",
    0,
    ProcessPriority::High,
    Credentials::new(
        SecurityLabel::confidential(),
        CapabilitySet::ipc(),
        IsolationLevel::Process,
    ),
    [Some(ServiceId::L2Subkernel), None],
    Some(ExecServiceDaemon::L2Driver),
    Some(ExecSignatureMetadata::new(
        "mirage-driver-root",
        0x4e564d45445f444d,
    )),
);

/// AHCI daemon descriptor; receives only its scoped PCI/MMIO/DMA/IRQ capabilities from L2.
pub const AHCID_SERVICE: ServiceDescriptor = ServiceDescriptor::new(
    ServiceId::Ahcid,
    "ahcid",
    0,
    ProcessPriority::High,
    Credentials::new(
        SecurityLabel::confidential(),
        CapabilitySet::ipc(),
        IsolationLevel::Process,
    ),
    [Some(ServiceId::L2Subkernel), None],
    Some(ExecServiceDaemon::L2Driver),
    Some(ExecSignatureMetadata::new(
        "mirage-driver-root",
        0x41484349445f444d,
    )),
);

/// AMDGPU display daemon descriptor; receives scoped PCI/MMIO/DMA/IRQ display hardware authority.
pub const AMDGPU_DISPLAYD_SERVICE: ServiceDescriptor = ServiceDescriptor::new(
    ServiceId::AmdgpuDisplayd,
    "amdgpu-displayd",
    0,
    ProcessPriority::High,
    Credentials::new(
        SecurityLabel::internal(),
        CapabilitySet::ipc(),
        IsolationLevel::Process,
    ),
    [Some(ServiceId::L2Subkernel), None],
    Some(ExecServiceDaemon::L2Driver),
    Some(ExecSignatureMetadata::new(
        "mirage-driver-root",
        0x414d444750554450,
    )),
);

/// Network daemon descriptor; device-facing and dependent on L2 authorization.
pub const NETWORKD_SERVICE: ServiceDescriptor = ServiceDescriptor::new(
    ServiceId::Networkd,
    "networkd",
    0,
    ProcessPriority::High,
    Credentials::new(
        SecurityLabel::internal(),
        CapabilitySet::ipc(),
        IsolationLevel::Process,
    ),
    [Some(ServiceId::L2Subkernel), None],
    Some(ExecServiceDaemon::Network),
    Some(ExecSignatureMetadata::new(
        "mirage-service-root",
        0x4e4554574f524b44,
    )),
);

/// Input daemon descriptor; device-facing and dependent on L2 authorization.
pub const INPUTD_SERVICE: ServiceDescriptor = ServiceDescriptor::new(
    ServiceId::Inputd,
    "inputd",
    0,
    ProcessPriority::High,
    Credentials::new(
        SecurityLabel::internal(),
        CapabilitySet::ipc(),
        IsolationLevel::Process,
    ),
    [Some(ServiceId::L2Subkernel), None],
    Some(ExecServiceDaemon::Input),
    Some(ExecSignatureMetadata::new(
        "mirage-service-root",
        0x494e50555444414d,
    )),
);

/// Built-in L1 manifest. The manifest order places L2 first, while dependency
/// checks also enforce this order if descriptors are rearranged.
pub const DEFAULT_STARTUP_MANIFEST: ServiceManifest<MAX_STARTUP_SERVICES> = ServiceManifest::new(
    [
        Some(L2_SUBKERNEL_SERVICE),
        Some(STORAGED_SERVICE),
        Some(USBD_SERVICE),
        Some(NVMED_SERVICE),
        Some(AHCID_SERVICE),
        Some(DISPLAYD_SERVICE),
        Some(AMDGPU_DISPLAYD_SERVICE),
        Some(NETWORKD_SERVICE),
        Some(INPUTD_SERVICE),
    ],
    MAX_STARTUP_SERVICES,
);

fn default_capability_specs(
    descriptor: ServiceDescriptor,
) -> [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES] {
    let mut capabilities = [None; MAX_SERVICE_CAPABILITIES];
    let mut idx = 0usize;

    match descriptor.id {
        ServiceId::L2Subkernel => {
            assign_capability(
                &mut capabilities,
                &mut idx,
                CapabilityObject::ServiceControl,
                CapabilityRights::service_control(),
            );
            assign_capability(
                &mut capabilities,
                &mut idx,
                CapabilityObject::ModuleLoad,
                CapabilityRights::service_control(),
            );
            assign_capability(
                &mut capabilities,
                &mut idx,
                CapabilityObject::ProcessHandle(ProcessId::new(u64::MAX)),
                CapabilityRights::process_control(),
            );
            assign_capability(
                &mut capabilities,
                &mut idx,
                CapabilityObject::MemoryObject(u64::MAX),
                CapabilityRights::memory(),
            );
        }
        ServiceId::Storaged => {
            assign_service_endpoint_capability(
                &mut capabilities,
                &mut idx,
                RegistryServiceId::Storaged,
            );
        }
        ServiceId::Usbd => {
            assign_service_endpoint_capability(
                &mut capabilities,
                &mut idx,
                RegistryServiceId::Usbd,
            );
            assign_usb_controller_capabilities(&mut capabilities, &mut idx);
        }
        ServiceId::Nvmed => {
            assign_service_endpoint_capability(
                &mut capabilities,
                &mut idx,
                RegistryServiceId::Nvmed,
            );
            assign_nvme_capability_bundle(
                &mut capabilities,
                &mut idx,
                HardwareCapabilityPolicy::new(7, 0xfed0_0000, 0x20_000)
                    .with_dma(0x8000_0000, 0x20_000)
                    .with_irq(46),
            );
        }
        ServiceId::Ahcid => {
            assign_service_endpoint_capability(
                &mut capabilities,
                &mut idx,
                RegistryServiceId::Ahcid,
            );
            assign_ahci_capability_bundle(
                &mut capabilities,
                &mut idx,
                HardwareCapabilityPolicy::new(7, 0xfed2_0000, 0x20_000)
                    .with_dma(0x8002_0000, 0x20_000)
                    .with_irq(47),
            );
        }
        ServiceId::Displayd => {
            assign_service_endpoint_capability(
                &mut capabilities,
                &mut idx,
                RegistryServiceId::Displayd,
            );
            assign_capability(
                &mut capabilities,
                &mut idx,
                CapabilityObject::MemoryObject(0x000b_8000),
                CapabilityRights::memory(),
            );
        }
        ServiceId::AmdgpuDisplayd => {
            assign_service_endpoint_capability(
                &mut capabilities,
                &mut idx,
                RegistryServiceId::AmdgpuDisplayd,
            );
            assign_amdgpu_capability_bundle(
                &mut capabilities,
                &mut idx,
                HardwareCapabilityPolicy::new(3, 0xe000_0000, 0x100_0000)
                    .with_dma(0x9000_0000, 0x100_000)
                    .with_irq(48)
                    .with_vram(0xc000_0000, 0x1000_0000)
                    .with_framebuffer(0xd000_0000, 0x0100_0000),
            );
        }
        ServiceId::Networkd => {
            assign_service_endpoint_capability(
                &mut capabilities,
                &mut idx,
                RegistryServiceId::Networkd,
            );
            assign_pci_mmio_dma_irq_capabilities(
                &mut capabilities,
                &mut idx,
                6,
                0xfed4_0000,
                0x10_000,
                49,
            );
        }
        ServiceId::Inputd => {
            assign_service_endpoint_capability(
                &mut capabilities,
                &mut idx,
                RegistryServiceId::Inputd,
            );
            assign_pci_mmio_dma_irq_capabilities(
                &mut capabilities,
                &mut idx,
                2,
                0xfed5_0000,
                0x10_000,
                50,
            );
        }
    }

    capabilities
}

fn assign_capability(
    capabilities: &mut [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES],
    idx: &mut usize,
    object: CapabilityObject,
    rights: CapabilityRights,
) {
    if *idx < MAX_SERVICE_CAPABILITIES {
        capabilities[*idx] = Some(ServiceCapabilityAssignment::new(object, rights));
        *idx += 1;
    }
}

fn assign_service_endpoint_capability(
    capabilities: &mut [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES],
    idx: &mut usize,
    service: RegistryServiceId,
) {
    assign_capability(
        capabilities,
        idx,
        CapabilityObject::IpcEndpoint(ProcessId::new(service.raw())),
        CapabilityRights::ipc_endpoint(),
    );
}

pub fn assign_nvme_capability_bundle(
    capabilities: &mut [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES],
    idx: &mut usize,
    policy: HardwareCapabilityPolicy,
) {
    assign_pci_mmio_dma_irq_register_bundle(capabilities, idx, policy, true);
    assign_capability(
        capabilities,
        idx,
        CapabilityObject::BlockDeviceRegistry,
        CapabilityRights::service_control(),
    );
}

pub fn assign_ahci_capability_bundle(
    capabilities: &mut [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES],
    idx: &mut usize,
    policy: HardwareCapabilityPolicy,
) {
    assign_pci_mmio_dma_irq_register_bundle(capabilities, idx, policy, true);
    assign_capability(
        capabilities,
        idx,
        CapabilityObject::BlockDeviceRegistry,
        CapabilityRights::service_control(),
    );
}

pub fn assign_xhci_usb_capability_bundle(
    capabilities: &mut [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES],
    idx: &mut usize,
    policy: HardwareCapabilityPolicy,
) {
    assign_pci_mmio_dma_irq_register_bundle(capabilities, idx, policy, true);
    assign_capability(
        capabilities,
        idx,
        CapabilityObject::HotplugController(policy.pci_device),
        CapabilityRights::service_control(),
    );
    assign_capability(
        capabilities,
        idx,
        CapabilityObject::BlockDeviceRegistry,
        CapabilityRights::service_control(),
    );
}

pub fn assign_amdgpu_capability_bundle(
    capabilities: &mut [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES],
    idx: &mut usize,
    policy: HardwareCapabilityPolicy,
) {
    assign_pci_mmio_dma_irq_register_bundle(capabilities, idx, policy, false);
    if let (Some(base), Some(length)) = (policy.vram_base, policy.vram_length) {
        assign_capability(
            capabilities,
            idx,
            CapabilityObject::VramRegion { base, length },
            CapabilityRights::memory(),
        );
    }
    if let (Some(base), Some(length)) = (policy.framebuffer_base, policy.framebuffer_length) {
        assign_capability(
            capabilities,
            idx,
            CapabilityObject::Framebuffer { base, length },
            CapabilityRights::memory(),
        );
    }
    assign_capability(
        capabilities,
        idx,
        CapabilityObject::DisplayRegistry,
        CapabilityRights::service_control(),
    );
}

fn assign_pci_mmio_dma_irq_register_bundle(
    capabilities: &mut [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES],
    idx: &mut usize,
    policy: HardwareCapabilityPolicy,
    require_dma: bool,
) {
    assign_capability(
        capabilities,
        idx,
        CapabilityObject::PciDevice(policy.pci_device),
        CapabilityRights::io(),
    );
    assign_capability(
        capabilities,
        idx,
        CapabilityObject::MmioRegion {
            base: policy.mmio_base,
            length: policy.mmio_length,
        },
        CapabilityRights::io(),
    );
    if require_dma || policy.dma_base.is_some() {
        if let (Some(base), Some(length)) = (policy.dma_base, policy.dma_length) {
            assign_capability(
                capabilities,
                idx,
                CapabilityObject::DmaRegion { base, length },
                CapabilityRights::io(),
            );
        }
    }
    if let Some(line) = policy.irq_line {
        assign_capability(
            capabilities,
            idx,
            CapabilityObject::IrqLine(line),
            CapabilityRights::io(),
        );
    }
}

fn assign_pci_mmio_dma_irq_capabilities(
    capabilities: &mut [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES],
    idx: &mut usize,
    pci_device: u64,
    mmio_base: u64,
    mmio_length: u64,
    irq_line: u16,
) {
    assign_pci_mmio_dma_irq_register_bundle(
        capabilities,
        idx,
        HardwareCapabilityPolicy::new(pci_device, mmio_base, mmio_length)
            .with_dma(mmio_base, mmio_length)
            .with_irq(irq_line),
        true,
    );
}

fn assign_usb_controller_capabilities(
    capabilities: &mut [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES],
    idx: &mut usize,
) {
    assign_xhci_usb_capability_bundle(
        capabilities,
        idx,
        HardwareCapabilityPolicy::new(2, 0xfed6_0000, 0x20_000)
            .with_dma(0x8006_0000, 0x20_000)
            .with_irq(51),
    );
}

fn reset_spawned_service_capabilities<const NPROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<NPROC, MSG_DEPTH>,
    pid: ProcessId,
) -> Result<(), KernelError> {
    kernel.revoke_task_capabilities(pid);
    kernel.grant_task_capability(
        pid,
        CapabilityObject::IpcEndpoint(pid),
        CapabilityRights::ipc_endpoint(),
    )?;
    Ok(())
}

/// Validate static service-daemon signature metadata embedded in the signed boot
/// module manifest. This models the signed-module gate for displayd, networkd,
/// inputd, and future supervised driver services before they are launched.
pub fn service_manifest_signature_valid(descriptor: ServiceDescriptor) -> bool {
    match descriptor.service_daemon {
        Some(ExecServiceDaemon::Display) => matches!(
            descriptor.signature,
            Some(ExecSignatureMetadata {
                signer: "mirage-service-root",
                manifest_digest: 0x444953504c415944
            })
        ),
        Some(ExecServiceDaemon::Network) => matches!(
            descriptor.signature,
            Some(ExecSignatureMetadata {
                signer: "mirage-service-root",
                manifest_digest: 0x4e4554574f524b44
            })
        ),
        Some(ExecServiceDaemon::Input) => matches!(
            descriptor.signature,
            Some(ExecSignatureMetadata {
                signer: "mirage-service-root",
                manifest_digest: 0x494e50555444414d
            })
        ),
        Some(ExecServiceDaemon::L2Driver) => matches!(
            descriptor.signature,
            Some(ExecSignatureMetadata {
                signer: "mirage-driver-root",
                ..
            })
        ),
        None => true,
    }
}

/// Dependency resolution result for one service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DependencyStatus {
    Ready(Option<ProcessId>),
    Waiting,
    Failed,
}

fn dependencies_ready<const CAP: usize>(
    descriptor: ServiceDescriptor,
    report: &ServiceStartupReport<CAP>,
) -> DependencyStatus {
    let mut parent = None;
    let mut dep_idx = 0usize;
    while dep_idx < MAX_SERVICE_DEPENDENCIES {
        if let Some(dependency) = descriptor.dependencies[dep_idx] {
            match report.dependency_state(dependency) {
                Some(StartupState::Running) => {
                    if parent.is_none() {
                        parent = report.dependency_pid(dependency);
                    }
                }
                Some(StartupState::Denied)
                | Some(StartupState::Crashed)
                | Some(StartupState::Stopped)
                | None => {
                    return DependencyStatus::Failed;
                }
                Some(StartupState::Created)
                | Some(StartupState::Validating)
                | Some(StartupState::Launching)
                | Some(StartupState::Restarting) => {
                    return DependencyStatus::Waiting;
                }
            }
        }
        dep_idx += 1;
    }
    DependencyStatus::Ready(parent)
}

/// Minimal boot services registered before any hardware driver or POSIX/QFS work.
pub const MINIMAL_CORE_SERVICES: [RegistryServiceId; 4] = [
    RegistryServiceId::Kernel,
    RegistryServiceId::Supervisor,
    RegistryServiceId::Console,
    RegistryServiceId::Memory,
];

/// Stub recovery manager state for the minimal boot path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryManagerStub {
    initialized: bool,
}

impl RecoveryManagerStub {
    pub const fn new() -> Self {
        Self { initialized: false }
    }

    pub const fn initialized() -> Self {
        Self { initialized: true }
    }

    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }
}

/// Stub driver manager state for minimal boot; it deliberately starts no drivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverManagerStub {
    initialized: bool,
    drivers_started: usize,
}

impl DriverManagerStub {
    pub const fn new() -> Self {
        Self {
            initialized: false,
            drivers_started: 0,
        }
    }

    pub const fn initialized_without_drivers() -> Self {
        Self {
            initialized: true,
            drivers_started: 0,
        }
    }

    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub const fn drivers_started(&self) -> usize {
        self.drivers_started
    }
}

/// One core service registration completed by the minimal supervisor path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MinimalServiceRegistration {
    pub service: RegistryServiceId,
    pub owner: ProcessId,
}

/// Minimal boot report separate from the full signed driver startup manifest.
#[derive(Clone, Copy, Debug)]
pub struct MinimalSupervisorReport {
    pub service_registry_initialized: bool,
    pub capability_table_initialized: bool,
    pub recovery_manager: RecoveryManagerStub,
    pub driver_manager: DriverManagerStub,
    pub supervisor_pid: Option<ProcessId>,
    registrations: [Option<MinimalServiceRegistration>; MINIMAL_CORE_SERVICES.len()],
    len: usize,
    pub failure: Option<KernelError>,
}

impl MinimalSupervisorReport {
    pub const fn new() -> Self {
        Self {
            service_registry_initialized: false,
            capability_table_initialized: false,
            recovery_manager: RecoveryManagerStub::new(),
            driver_manager: DriverManagerStub::new(),
            supervisor_pid: None,
            registrations: [None; MINIMAL_CORE_SERVICES.len()],
            len: 0,
            failure: None,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub fn registration(&self, index: usize) -> Option<MinimalServiceRegistration> {
        if index < self.len {
            self.registrations[index]
        } else {
            None
        }
    }

    fn push_registration(&mut self, service: RegistryServiceId, owner: ProcessId) {
        if self.len < self.registrations.len() {
            self.registrations[self.len] = Some(MinimalServiceRegistration { service, owner });
            self.len += 1;
        }
    }
}

/// Supervisor-owned platform initialization report.
///
/// `mirage-platform` supplies discovery facts and device candidates only; this
/// report is intentionally held by the supervisor so capability grant, service
/// launch, and recovery policy remain centralized here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupervisorPlatformReport {
    pub platform: PlatformInfo,
}

impl SupervisorPlatformReport {
    pub fn device_discovery_events(&self) -> &[DeviceDiscoveryEvent] {
        &self.platform.events
    }
}

/// Supervisor entry point for early service lifecycle management.
#[derive(Clone, Copy, Debug, Default)]
pub struct Supervisor;

impl Supervisor {
    pub const fn new() -> Self {
        Self
    }

    /// Forward a supervisor-approved termination request through MTSS.
    ///
    /// This keeps service shutdown policy in the supervisor while leaving task
    /// lifecycle mechanics behind the MTSS/kernel boundary.
    pub fn terminate_policy_approved_task<const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        task: ProcessId,
    ) -> Result<(), KernelError> {
        kernel.mtss_terminate_task(task)
    }

    /// Forward a supervisor-approved future priority request through MTSS.
    ///
    /// Priority policy may be evaluated here, but the transition itself belongs
    /// to MTSS and later kernel/backend execution. Current backends return a
    /// placeholder error until priority mutation is exposed.
    pub fn request_task_priority<const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        request: SupervisorMtssPriorityRequest,
    ) -> Result<(), KernelError> {
        kernel.mtss_request_priority(request)
    }

    /// Initialize platform discovery without granting driver authority.
    ///
    /// The platform service reports CPU, Ryzen generation, timer, interrupt,
    /// chipset, and IOMMU candidates. The supervisor keeps ownership of policy,
    /// recovery, and capability issuance.
    pub fn init_platform(&self) -> SupervisorPlatformReport {
        SupervisorPlatformReport {
            platform: PlatformService::detect().into_info(),
        }
    }

    /// Mint a bounded hardware-capability proposal for a discovered device.
    ///
    /// Driver services such as AHCI, NVMe, USB, and AMDGPU claim devices through
    /// these supervisor-granted capabilities. They do not receive authority from
    /// Ryzen generation classification and should not depend on `mirage-ryzen`
    /// for device ownership.
    pub fn grant_platform_caps(
        &self,
        event: DeviceDiscoveryEvent,
    ) -> [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES] {
        let mut capabilities = [None; MAX_SERVICE_CAPABILITIES];
        let mut idx = 0usize;
        let pci = event.pci();
        let pci_device = pci.capability_object_id();
        let mmio_base = pci.bar0_base.unwrap_or(0);
        let mmio_length = pci.bar0_length.unwrap_or(0x1000);
        let mut policy = HardwareCapabilityPolicy::new(pci_device, mmio_base, mmio_length);
        if let Some(irq_line) = pci.irq_line {
            policy = policy.with_irq(irq_line);
        }

        match event {
            DeviceDiscoveryEvent::DriverCandidate { role, .. } => match role {
                DeviceCandidateRole::AhciStorage => {
                    assign_ahci_capability_bundle(&mut capabilities, &mut idx, policy);
                }
                DeviceCandidateRole::NvmeStorage => {
                    assign_nvme_capability_bundle(&mut capabilities, &mut idx, policy);
                }
                DeviceCandidateRole::XhciUsb => {
                    assign_xhci_usb_capability_bundle(&mut capabilities, &mut idx, policy);
                }
                DeviceCandidateRole::AmdGpuDisplay => {
                    assign_amdgpu_capability_bundle(&mut capabilities, &mut idx, policy);
                }
                DeviceCandidateRole::AmdIommu => {
                    assign_pci_mmio_dma_irq_register_bundle(
                        &mut capabilities,
                        &mut idx,
                        policy,
                        false,
                    );
                    assign_capability(
                        &mut capabilities,
                        &mut idx,
                        CapabilityObject::ServiceControl,
                        CapabilityRights::service_control(),
                    );
                }
            },
            DeviceDiscoveryEvent::IommuCapability { .. } => {
                assign_pci_mmio_dma_irq_register_bundle(&mut capabilities, &mut idx, policy, false);
            }
        }

        capabilities
    }

    /// Return supervisor-visible device discovery events from a platform report.
    pub fn device_discovery_events<'a>(
        &self,
        report: &'a SupervisorPlatformReport,
    ) -> &'a [DeviceDiscoveryEvent] {
        report.device_discovery_events()
    }

    /// Initialize the minimal supervisor path without launching the full driver manifest.
    ///
    /// This path sets up/stubs the service registry, capability table, recovery
    /// manager, and driver manager, then registers only the core kernel-facing
    /// services needed for a bootable skeleton. It deliberately does not start
    /// NVMe, AHCI, USB, AMDGPU, QFS, POSIX, or libc work.
    pub fn bootstrap_minimal<const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
    ) -> MinimalSupervisorReport {
        let mut report = MinimalSupervisorReport::new();
        report.service_registry_initialized = true;
        report.recovery_manager = RecoveryManagerStub::initialized();
        report.driver_manager = DriverManagerStub::initialized_without_drivers();

        let supervisor_pid = match kernel.spawn_initial_process(Credentials::system()) {
            Ok(pid) => pid,
            Err(error) => {
                report.failure = Some(error);
                return report;
            }
        };

        report.supervisor_pid = Some(supervisor_pid);
        report.capability_table_initialized = true;

        let mut idx = 0usize;
        while idx < MINIMAL_CORE_SERVICES.len() {
            let service = MINIMAL_CORE_SERVICES[idx];
            match kernel.register_endpoint(supervisor_pid, service, supervisor_pid) {
                Ok(()) => {
                    crate::kprintln!("supervisor: registered core service '{}'", service.name());
                    report.push_registration(service, supervisor_pid);
                }
                Err(error) => {
                    report.failure = Some(error);
                    return report;
                }
            }
            idx += 1;
        }

        report
    }

    /// Launch a mock service admitted by external boot manifest validation.
    pub fn launch_mock_manifest_service<const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        service: mock_service::MockManifestService<'_>,
    ) -> Result<mock_service::MockServiceLaunchReport, mock_service::MockServiceLaunchError> {
        mock_service::launch_echo_service_from_validated_manifest(kernel, service)
    }

    /// Recover a manifest-launched mock echo service through supervisor crash policy.
    pub fn recover_mock_manifest_service_crash<const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        previous: &mock_service::MockServiceLaunchReport,
        service: mock_service::MockManifestService<'_>,
        exit: ProcessExitReport,
    ) -> Result<mock_service::MockServiceRecoveryReport, mock_service::MockServiceLaunchError> {
        mock_service::recover_echo_service_after_crash(kernel, previous, service, exit)
    }

    /// Run one echo request/reply transaction through the registered `echo.ipc` endpoint.
    pub fn dispatch_echo_request<const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        report: &mock_service::MockServiceLaunchReport,
        caller: ProcessId,
        payload: crate::kernel::ipc::MessagePayload,
    ) -> Result<crate::kernel::ipc::MessagePayload, mock_service::MockServiceLaunchError> {
        mock_service::dispatch_echo_request(kernel, report, caller, payload)
    }

    /// Start the built-in services in dependency order.
    pub fn bootstrap_services<const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
    ) -> DefaultServiceStartupReport {
        self.spawn_services(kernel, &DEFAULT_STARTUP_MANIFEST)
    }

    /// Start services from a signed manifest using only kernel mechanism primitives.
    pub fn spawn_services<const CAP: usize, const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        manifest: &ServiceManifest<CAP>,
    ) -> ServiceStartupReport<CAP> {
        let mut report = ServiceStartupReport::from_manifest(manifest);

        loop {
            let mut made_progress = false;
            let mut pending = 0usize;
            let mut idx = 0usize;

            while idx < report.len() {
                if let Some(record) = report.record(idx) {
                    if record.state == StartupState::Created {
                        match dependencies_ready(record.descriptor, &report) {
                            DependencyStatus::Ready(parent) => {
                                report.set_validating(idx);
                                if !service_manifest_signature_valid(record.descriptor) {
                                    report.set_denied(
                                        idx,
                                        KernelError::SecurityViolation(
                                            IsolationError::PolicyViolation,
                                        ),
                                    );
                                    made_progress = true;
                                    idx += 1;
                                    continue;
                                }

                                report.set_launching(idx);
                                let request = SpawnTaskRequest {
                                    parent,
                                    entry_point: record.descriptor.entry_point,
                                    priority: record.descriptor.priority,
                                    credentials: record.descriptor.credentials,
                                };

                                match kernel.mtss_spawn_task(request) {
                                    Ok(pid) => {
                                        if let Err(error) =
                                            reset_spawned_service_capabilities(kernel, pid)
                                        {
                                            report.set_denied(idx, error);
                                            made_progress = true;
                                            idx += 1;
                                            continue;
                                        }
                                        report.set_running(idx, pid);
                                        if let Err(error) =
                                            report.regrant_record_capabilities(kernel, idx, pid)
                                        {
                                            report.set_denied(idx, error);
                                            made_progress = true;
                                            idx += 1;
                                            continue;
                                        }
                                        if let Some(registry_service) =
                                            startup_service_to_registry(record.descriptor.id)
                                        {
                                            report.record_endpoint_claim(
                                                record.descriptor.id,
                                                registry_service,
                                            );
                                            if let Some(authorizer) = parent {
                                                let _ = kernel.register_endpoint(
                                                    authorizer,
                                                    registry_service,
                                                    pid,
                                                );
                                            }
                                        }
                                    }
                                    Err(error) => report.set_denied(idx, error),
                                }
                                made_progress = true;
                            }
                            DependencyStatus::Waiting => {
                                pending += 1;
                            }
                            DependencyStatus::Failed => {
                                report.set_denied(idx, KernelError::InvalidArgument);
                                made_progress = true;
                            }
                        }
                    }
                }
                idx += 1;
            }

            if pending == 0 {
                break;
            }

            if !made_progress {
                let mut fail_idx = 0usize;
                while fail_idx < report.len() {
                    if let Some(record) = report.record(fail_idx) {
                        if record.state == StartupState::Created {
                            report.set_denied(fail_idx, KernelError::InvalidArgument);
                        }
                    }
                    fail_idx += 1;
                }
                break;
            }
        }

        report
    }
}

fn startup_service_to_registry(service: ServiceId) -> Option<RegistryServiceId> {
    match service {
        ServiceId::Storaged => Some(RegistryServiceId::Storaged),
        ServiceId::Usbd => Some(RegistryServiceId::Usbd),
        ServiceId::Nvmed => Some(RegistryServiceId::Nvmed),
        ServiceId::Ahcid => Some(RegistryServiceId::Ahcid),
        ServiceId::Displayd => Some(RegistryServiceId::Displayd),
        ServiceId::AmdgpuDisplayd => Some(RegistryServiceId::AmdgpuDisplayd),
        ServiceId::Networkd => Some(RegistryServiceId::Networkd),
        ServiceId::Inputd => Some(RegistryServiceId::Inputd),
        ServiceId::L2Subkernel => None,
    }
}

pub type DefaultServiceStartupReport = ServiceStartupReport<MAX_STARTUP_SERVICES>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subkernel::SecurityClass;

    fn assigned_capability(
        report: &DefaultServiceStartupReport,
        service: ServiceId,
        object: CapabilityObject,
    ) -> Option<ServiceCapabilityAssignment> {
        let record = report.find(service)?;
        let mut idx = 0usize;
        while idx < MAX_SERVICE_CAPABILITIES {
            if let Some(capability) = record.assigned_capabilities[idx] {
                if capability.object == object {
                    return Some(capability);
                }
            }
            idx += 1;
        }
        None
    }

    #[test]
    fn static_driver_bundles_include_service_specific_authority() {
        let report = ServiceStartupReport::from_manifest(&DEFAULT_STARTUP_MANIFEST);

        assert!(assigned_capability(
            &report,
            ServiceId::Nvmed,
            CapabilityObject::BlockDeviceRegistry,
        )
        .is_some());
        assert!(assigned_capability(
            &report,
            ServiceId::Usbd,
            CapabilityObject::HotplugController(2),
        )
        .is_some());
        assert!(assigned_capability(
            &report,
            ServiceId::Usbd,
            CapabilityObject::BlockDeviceRegistry,
        )
        .is_some());
        assert!(assigned_capability(
            &report,
            ServiceId::AmdgpuDisplayd,
            CapabilityObject::VramRegion {
                base: 0xc000_0000,
                length: 0x1000_0000,
            },
        )
        .is_some());
        assert!(assigned_capability(
            &report,
            ServiceId::AmdgpuDisplayd,
            CapabilityObject::Framebuffer {
                base: 0xd000_0000,
                length: 0x0100_0000,
            },
        )
        .is_some());
        assert!(assigned_capability(
            &report,
            ServiceId::AmdgpuDisplayd,
            CapabilityObject::DisplayRegistry,
        )
        .is_some());
    }

    #[test]
    fn manifest_signature_validation_failure_marks_service_denied() {
        const BAD_DISPLAYD: ServiceDescriptor = ServiceDescriptor::new(
            ServiceId::Displayd,
            "displayd-bad-signature",
            0,
            ProcessPriority::High,
            Credentials::new(
                SecurityLabel::internal(),
                CapabilitySet::ipc(),
                IsolationLevel::Process,
            ),
            [None, None],
            Some(ExecServiceDaemon::Display),
            Some(ExecSignatureMetadata::new(
                "mirage-service-root",
                0xdead_beef,
            )),
        );
        const BAD_MANIFEST: ServiceManifest<1> = ServiceManifest::new([Some(BAD_DISPLAYD)], 1);

        let mut kernel = Kernel::<4, 4>::new();
        kernel.bootstrap();
        let supervisor = Supervisor::new();

        let report = supervisor.spawn_services(&mut kernel, &BAD_MANIFEST);

        assert_eq!(
            report.state(ServiceId::Displayd),
            Some(StartupState::Denied)
        );
        assert_eq!(report.pid(ServiceId::Displayd), None);
        assert!(matches!(
            report.record(0).and_then(|record| record.failure),
            Some(KernelError::SecurityViolation(
                IsolationError::PolicyViolation
            ))
        ));
    }

    #[test]
    fn restart_policy_never_stops_after_revoke_without_regranting_hardware() {
        let mut kernel = Kernel::<16, 4>::new();
        kernel.bootstrap();
        let supervisor = Supervisor::new();
        let mut report = supervisor.bootstrap_services(&mut kernel);
        let nvmed = report.pid(ServiceId::Nvmed).unwrap();
        let index = report.find_index(ServiceId::Nvmed).unwrap();
        let old_irq = assigned_capability(&report, ServiceId::Nvmed, CapabilityObject::IrqLine(46))
            .and_then(|capability| capability.id)
            .unwrap();

        let mut record = report.records[index].unwrap();
        record.restart_policy = RestartPolicy::Never;
        report.records[index] = Some(record);

        let exit = kernel
            .exit_process(nvmed, crate::kernel::process::ExitStatus::signaled(9))
            .unwrap();
        let recovery = report.handle_process_exit(&mut kernel, exit);

        assert!(matches!(recovery, ServiceRecoveryState::Stopped));
        assert_eq!(report.pid(ServiceId::Nvmed), None);
        assert_eq!(report.state(ServiceId::Nvmed), Some(StartupState::Stopped));
        assert!(matches!(
            kernel.revoke_task_capability(old_irq),
            Err(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            ))
        ));
        assert_eq!(
            assigned_capability(&report, ServiceId::Nvmed, CapabilityObject::IrqLine(46))
                .and_then(|capability| capability.id),
            None
        );
    }

    #[test]
    fn recovers_nvmed_and_reissues_scoped_hardware_capabilities() {
        let mut kernel = Kernel::<16, 4>::new();
        kernel.bootstrap();
        let supervisor = Supervisor::new();
        let mut report = supervisor.bootstrap_services(&mut kernel);
        let old_nvmed = report.pid(ServiceId::Nvmed).unwrap();

        report.record_device_claim(ServiceId::Nvmed, RegistryServiceId::Nvmed, DeviceId::new(7));
        kernel
            .claim_service_device(old_nvmed, RegistryServiceId::Nvmed, DeviceId::new(7))
            .unwrap();

        let old_dma = assigned_capability(
            &report,
            ServiceId::Nvmed,
            CapabilityObject::DmaRegion {
                base: 0x8000_0000,
                length: 0x20_000,
            },
        )
        .and_then(|capability| capability.id)
        .unwrap();
        let old_mmio = assigned_capability(
            &report,
            ServiceId::Nvmed,
            CapabilityObject::MmioRegion {
                base: 0xfed0_0000,
                length: 0x20_000,
            },
        )
        .and_then(|capability| capability.id)
        .unwrap();

        let exit = kernel
            .exit_process(old_nvmed, crate::kernel::process::ExitStatus::signaled(11))
            .unwrap();
        let recovery = report.handle_process_exit(&mut kernel, exit);

        let ServiceRecoveryState::Restarted(new_nvmed) = recovery else {
            panic!("nvmed was not restarted: {:?}", recovery);
        };
        assert_ne!(new_nvmed, old_nvmed);
        assert_eq!(report.pid(ServiceId::Nvmed), Some(new_nvmed));
        assert_eq!(report.state(ServiceId::Nvmed), Some(StartupState::Running));
        assert_eq!(report.generation(ServiceId::Nvmed), Some(1));
        assert_eq!(
            kernel.service_owner(RegistryServiceId::Nvmed),
            Some(new_nvmed)
        );

        assert!(matches!(
            kernel.revoke_task_capability(old_dma),
            Err(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            ))
        ));
        assert!(matches!(
            kernel.revoke_task_capability(old_mmio),
            Err(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            ))
        ));

        let new_dma = assigned_capability(
            &report,
            ServiceId::Nvmed,
            CapabilityObject::DmaRegion {
                base: 0x8000_0000,
                length: 0x20_000,
            },
        )
        .and_then(|capability| capability.id)
        .unwrap();
        let new_mmio = assigned_capability(
            &report,
            ServiceId::Nvmed,
            CapabilityObject::MmioRegion {
                base: 0xfed0_0000,
                length: 0x20_000,
            },
        )
        .and_then(|capability| capability.id)
        .unwrap();
        assert_ne!(new_dma, old_dma);
        assert_ne!(new_mmio, old_mmio);

        let mut buffer = [0u8; 64];
        assert!(matches!(
            kernel.device_read(new_nvmed, DeviceId::new(7), &mut buffer),
            Ok(_) | Err(KernelError::DeviceFault(_))
        ));
    }

    #[test]
    fn recovers_displayd_and_restores_endpoint_registration() {
        let mut kernel = Kernel::<16, 4>::new();
        kernel.bootstrap();
        let supervisor = Supervisor::new();
        let mut report = supervisor.bootstrap_services(&mut kernel);
        let old_displayd = report.pid(ServiceId::Displayd).unwrap();

        assert_eq!(
            kernel.service_owner(RegistryServiceId::Displayd),
            Some(old_displayd)
        );

        let exit = kernel
            .exit_process(
                old_displayd,
                crate::kernel::process::ExitStatus::signaled(6),
            )
            .unwrap();
        let recovery = report.handle_process_exit(&mut kernel, exit);

        let ServiceRecoveryState::Restarted(new_displayd) = recovery else {
            panic!("displayd was not restarted: {:?}", recovery);
        };
        assert_ne!(new_displayd, old_displayd);
        assert_eq!(report.pid(ServiceId::Displayd), Some(new_displayd));
        assert_eq!(
            report.state(ServiceId::Displayd),
            Some(StartupState::Running)
        );
        assert_eq!(report.generation(ServiceId::Displayd), Some(1));
        assert_eq!(
            kernel.service_owner(RegistryServiceId::Displayd),
            Some(new_displayd)
        );
    }

    const ECHO_RIGHTS: [&str; 2] = ["SEND", "RECEIVE"];
    const ECHO_CAPABILITIES: [mock_service::MockManifestCapability<'static>; 1] =
        [mock_service::MockManifestCapability {
            object: mock_service::IPC_ENDPOINT_CAPABILITY_OBJECT,
            endpoint: Some(mock_service::ECHO_IPC_ENDPOINT),
            rights: &ECHO_RIGHTS,
        }];
    const ECHO_MANIFEST_SERVICE: mock_service::MockManifestService<'static> =
        mock_service::MockManifestService {
            module_id: mock_service::ECHO_SERVICE_MODULE_ID,
            image: mock_service::ECHO_SERVICE_IMAGE,
            restart_always: true,
            capabilities: &ECHO_CAPABILITIES,
        };

    fn launch_manifest_echo<const NPROC: usize, const MSG_DEPTH: usize>(
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
    ) -> mock_service::MockServiceLaunchReport {
        Supervisor::new()
            .launch_mock_manifest_service(kernel, ECHO_MANIFEST_SERVICE)
            .unwrap()
    }

    #[test]
    fn manifest_echo_ipc_is_denied_without_sender_capability() {
        let mut kernel = Kernel::<8, 4>::new();
        kernel.bootstrap();
        let report = launch_manifest_echo(&mut kernel);
        let caller = kernel.spawn_initial_process(Credentials::user()).unwrap();
        kernel.revoke_task_capabilities(caller);

        let payload = crate::kernel::ipc::MessagePayload::from_slice(
            SecurityClass::Internal,
            b"capability denied",
        );
        assert!(matches!(
            kernel.send_message(caller, report.service_pid, payload),
            Err(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            ))
        ));
        assert_eq!(kernel.receive_or_block(report.service_pid).unwrap(), None);
    }

    #[test]
    fn manifest_echo_ipc_is_allowed_with_sender_capability() {
        let mut kernel = Kernel::<8, 4>::new();
        kernel.bootstrap();
        let report = launch_manifest_echo(&mut kernel);
        let caller = kernel.spawn_initial_process(Credentials::user()).unwrap();
        kernel.revoke_task_capabilities(caller);
        kernel
            .grant_task_capability(
                caller,
                CapabilityObject::IpcEndpoint(caller),
                CapabilityRights::ipc_endpoint(),
            )
            .unwrap();
        kernel
            .grant_task_capability(
                caller,
                CapabilityObject::IpcEndpoint(report.service_pid),
                CapabilityRights::ipc_endpoint(),
            )
            .unwrap();

        let payload = crate::kernel::ipc::MessagePayload::from_slice(
            SecurityClass::Internal,
            b"capability allowed",
        );
        kernel
            .send_message(caller, report.service_pid, payload)
            .unwrap();

        let request = kernel
            .receive_or_block(report.service_pid)
            .unwrap()
            .unwrap();
        assert_eq!(request.sender, caller);
        assert_eq!(request.receiver, report.service_pid);
        assert_eq!(request.payload, payload);
    }

    #[test]
    fn manifest_echo_service_cannot_register_echo_ipc_without_receive_endpoint_capability() {
        let mut kernel = Kernel::<8, 4>::new();
        kernel.bootstrap();
        let supervisor_pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let service_pid = kernel
            .spawn_task(SpawnTaskRequest {
                parent: Some(supervisor_pid),
                entry_point: 0,
                priority: ProcessPriority::Normal,
                credentials: Credentials::user(),
            })
            .unwrap();
        kernel.revoke_task_capabilities(service_pid);

        assert!(matches!(
            kernel.register_endpoint(supervisor_pid, RegistryServiceId::EchoIpc, service_pid),
            Err(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            ))
        ));
        assert_eq!(kernel.service_owner(RegistryServiceId::EchoIpc), None);
    }

    #[test]
    fn manifest_echo_service_registers_echo_ipc_after_supervisor_grant() {
        let mut kernel = Kernel::<8, 4>::new();
        kernel.bootstrap();
        let supervisor_pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let service_pid = kernel
            .spawn_task(SpawnTaskRequest {
                parent: Some(supervisor_pid),
                entry_point: 0,
                priority: ProcessPriority::Normal,
                credentials: Credentials::user(),
            })
            .unwrap();
        kernel.revoke_task_capabilities(service_pid);
        kernel
            .grant_task_capability(
                service_pid,
                CapabilityObject::IpcEndpoint(service_pid),
                CapabilityRights::ipc_endpoint(),
            )
            .unwrap();

        kernel
            .register_endpoint(supervisor_pid, RegistryServiceId::EchoIpc, service_pid)
            .unwrap();
        assert_eq!(
            kernel.service_owner(RegistryServiceId::EchoIpc),
            Some(service_pid)
        );
    }

    #[test]
    fn manifest_echo_request_returns_payload_unchanged() {
        let mut kernel = Kernel::<8, 4>::new();
        kernel.bootstrap();
        let supervisor = Supervisor::new();
        let report = supervisor
            .launch_mock_manifest_service(&mut kernel, ECHO_MANIFEST_SERVICE)
            .unwrap();
        let caller = kernel.spawn_initial_process(Credentials::user()).unwrap();
        let payload = crate::kernel::ipc::MessagePayload::from_slice(
            SecurityClass::Internal,
            b"echo this payload",
        );

        let response = supervisor
            .dispatch_echo_request(&mut kernel, &report, caller, payload)
            .unwrap();

        assert_eq!(response, payload);
    }

    #[test]
    fn manifest_echo_capabilities_are_revoked_when_service_crashes() {
        let mut kernel = Kernel::<8, 4>::new();
        kernel.bootstrap();
        let report = launch_manifest_echo(&mut kernel);
        let old_endpoint_capability = report.endpoint_capability_id;

        kernel
            .exit_process(
                report.service_pid,
                crate::kernel::process::ExitStatus::signaled(11),
            )
            .unwrap();
        kernel.revoke_task_capabilities(report.service_pid);
        kernel.revoke_task(report.service_pid);
        kernel.revoke_service_owner(report.service_pid);

        assert!(matches!(
            kernel.revoke_task_capability(old_endpoint_capability),
            Err(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            ))
        ));
        assert_eq!(kernel.service_owner(RegistryServiceId::EchoIpc), None);
    }

    #[test]
    fn manifest_echo_restart_uses_fresh_capability_instead_of_revoked_authority() {
        let mut kernel = Kernel::<12, 4>::new();
        kernel.bootstrap();
        let first = launch_manifest_echo(&mut kernel);
        let old_service_pid = first.service_pid;
        let old_endpoint_capability = first.endpoint_capability_id;

        kernel
            .exit_process(
                old_service_pid,
                crate::kernel::process::ExitStatus::signaled(6),
            )
            .unwrap();
        kernel.revoke_task_capabilities(old_service_pid);
        kernel.revoke_task(old_service_pid);
        kernel.revoke_service_owner(old_service_pid);
        assert!(matches!(
            kernel.revoke_task_capability(old_endpoint_capability),
            Err(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            ))
        ));

        let restarted = launch_manifest_echo(&mut kernel);

        assert_ne!(restarted.service_pid, old_service_pid);
        assert_ne!(restarted.endpoint_capability_id, old_endpoint_capability);
        assert_eq!(
            kernel.service_owner(RegistryServiceId::EchoIpc),
            Some(restarted.service_pid)
        );
    }

    #[test]
    fn test_echo_service_crash_restart() {
        let manifest =
            mirage_boot::parse_manifest_toml(include_str!("../../boot/manifest.mock.toml"))
                .expect("boot/manifest.mock.toml parses");
        let plans = mirage_boot::build_service_launch_plan(&manifest)
            .expect("boot/manifest.mock.toml validates");
        let plan = plans
            .iter()
            .find(|plan| plan.module_id.as_str() == mock_service::ECHO_SERVICE_MODULE_ID)
            .expect("echo-service launch plan exists");
        assert_eq!(plan.restart, mirage_boot::RestartPolicy::Always);

        let rights: Vec<&str> = plan.capabilities[0]
            .rights
            .iter()
            .map(String::as_str)
            .collect();
        let capability = mock_service::MockManifestCapability {
            object: plan.capabilities[0].object.as_str(),
            endpoint: plan.capabilities[0].endpoint.as_deref(),
            rights: rights.as_slice(),
        };
        let capabilities = [capability];
        let service = mock_service::MockManifestService {
            module_id: plan.module_id.as_str(),
            image: plan.image.as_str(),
            restart_always: plan.restart == mirage_boot::RestartPolicy::Always,
            capabilities: &capabilities,
        };

        let mut kernel = Kernel::<16, 8>::new();
        kernel.bootstrap();
        let supervisor = Supervisor::new();
        let first = supervisor
            .launch_mock_manifest_service(&mut kernel, service)
            .expect("supervisor launches echo-service");
        let old_service_pid = first.service_pid;
        let old_endpoint_capability = first.endpoint_capability_id;

        assert!(first.service.is_registered());
        assert_eq!(first.endpoint, RegistryServiceId::EchoIpc);
        assert_eq!(
            kernel.service_owner(RegistryServiceId::EchoIpc),
            Some(old_service_pid)
        );

        let caller = kernel.spawn_initial_process(Credentials::user()).unwrap();
        let before_crash = crate::kernel::ipc::MessagePayload::from_slice(
            SecurityClass::Internal,
            b"echo before crash",
        );
        assert_eq!(
            supervisor
                .dispatch_echo_request(&mut kernel, &first, caller, before_crash)
                .expect("echo IPC works before crash"),
            before_crash
        );

        let exit = kernel
            .exit_process(
                old_service_pid,
                crate::kernel::process::ExitStatus::signaled(11),
            )
            .expect("crashed echo-service produces an exit report");
        let recovery = supervisor
            .recover_mock_manifest_service_crash(&mut kernel, &first, service, exit)
            .expect("restart=always relaunches echo-service");

        assert_eq!(recovery.crashed_state, StartupState::Crashed);
        assert_eq!(recovery.registry_owner_after_revoke, None);
        assert!(matches!(
            kernel.revoke_task_capability(old_endpoint_capability),
            Err(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            ))
        ));

        let restarted = recovery.restarted;
        assert_ne!(restarted.service_pid, old_service_pid);
        assert_ne!(restarted.endpoint_capability_id, old_endpoint_capability);
        assert!(restarted.service.is_registered());
        assert_eq!(
            kernel.service_owner(RegistryServiceId::EchoIpc),
            Some(restarted.service_pid)
        );

        let after_restart = crate::kernel::ipc::MessagePayload::from_slice(
            SecurityClass::Internal,
            b"echo after restart",
        );
        assert_eq!(
            supervisor
                .dispatch_echo_request(&mut kernel, &restarted, caller, after_restart)
                .expect("echo IPC works after restart"),
            after_restart
        );
    }

    #[test]
    fn default_manifest_blocks_device_daemons_until_l2_runs() {
        let mut report = ServiceStartupReport::from_manifest(&DEFAULT_STARTUP_MANIFEST);
        let l2 = DEFAULT_STARTUP_MANIFEST.descriptor(0).unwrap();
        let storaged = DEFAULT_STARTUP_MANIFEST.descriptor(1).unwrap();

        assert_eq!(
            dependencies_ready(l2, &report),
            DependencyStatus::Ready(None)
        );
        assert_eq!(
            dependencies_ready(storaged, &report),
            DependencyStatus::Waiting
        );

        report.set_running(0, ProcessId::new(1));

        assert_eq!(
            dependencies_ready(storaged, &report),
            DependencyStatus::Ready(Some(ProcessId::new(1)))
        );
    }
}
