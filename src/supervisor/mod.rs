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

/// Startup state for a service supervised by the policy broker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupState {
    Pending,
    Starting,
    Running,
    Failed,
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
pub const MAX_SERVICE_CAPABILITIES: usize = 8;

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
            state: StartupState::Pending,
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

        self.revoke_record_capabilities(kernel, index);
        kernel.revoke_task(report.pid);
        kernel.revoke_service_owner(report.pid);

        if let Some(mut record) = self.records[index] {
            record.pid = None;
            record.state = StartupState::Failed;
            record.last_exit_status = Some(report.status);
            record.failure = None;
            self.records[index] = Some(record);
        }

        let record = self.records[index].unwrap();
        if record.restart_policy == RestartPolicy::Never {
            return ServiceRecoveryState::Stopped;
        }

        match self.respawn_record(kernel, index) {
            Ok(pid) => ServiceRecoveryState::Restarted(pid),
            Err(error) => {
                self.set_failed(index, error);
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

    fn set_starting(&mut self, index: usize) {
        if let Some(mut record) = self.records[index] {
            record.state = StartupState::Starting;
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

    fn set_failed(&mut self, index: usize, error: KernelError) {
        if let Some(mut record) = self.records[index] {
            record.state = StartupState::Failed;
            record.pid = None;
            record.failure = Some(error);
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

    fn revoke_record_capabilities<const NPROC: usize, const MSG_DEPTH: usize>(
        &mut self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        index: usize,
    ) {
        if let Some(mut record) = self.records[index] {
            let mut cap_idx = 0usize;
            while cap_idx < MAX_SERVICE_CAPABILITIES {
                if let Some(capability) = record.assigned_capabilities[cap_idx] {
                    if let Some(id) = capability.id {
                        let _ = kernel.revoke_task_capability(id);
                    }
                    record.assigned_capabilities[cap_idx] = Some(capability.unassigned());
                }
                cap_idx += 1;
            }
            self.records[index] = Some(record);
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
        let pid = kernel.spawn_task(SpawnTaskRequest {
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
            assign_pci_mmio_dma_irq_capabilities(
                &mut capabilities,
                &mut idx,
                7,
                0xfed0_0000,
                0x20_000,
                46,
            );
        }
        ServiceId::Ahcid => {
            assign_service_endpoint_capability(
                &mut capabilities,
                &mut idx,
                RegistryServiceId::Ahcid,
            );
            assign_pci_mmio_dma_irq_capabilities(
                &mut capabilities,
                &mut idx,
                7,
                0xfed2_0000,
                0x20_000,
                47,
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
            assign_pci_mmio_dma_irq_capabilities(
                &mut capabilities,
                &mut idx,
                3,
                0xe000_0000,
                0x100_0000,
                48,
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

fn assign_pci_mmio_dma_irq_capabilities(
    capabilities: &mut [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES],
    idx: &mut usize,
    pci_device: u64,
    mmio_base: u64,
    mmio_length: u64,
    irq_line: u16,
) {
    assign_capability(
        capabilities,
        idx,
        CapabilityObject::PciDevice(pci_device),
        CapabilityRights::io(),
    );
    // Mirage models scoped MMIO windows with memory-object capabilities until
    // the lower-level capability object space grows a dedicated MMIO variant.
    assign_capability(
        capabilities,
        idx,
        CapabilityObject::MemoryObject(mmio_base),
        CapabilityRights::memory(),
    );
    assign_capability(
        capabilities,
        idx,
        CapabilityObject::DmaRegion {
            base: mmio_base,
            length: mmio_length,
        },
        CapabilityRights::io(),
    );
    assign_capability(
        capabilities,
        idx,
        CapabilityObject::IrqLine(irq_line),
        CapabilityRights::io(),
    );
}

fn assign_usb_controller_capabilities(
    capabilities: &mut [Option<ServiceCapabilityAssignment>; MAX_SERVICE_CAPABILITIES],
    idx: &mut usize,
) {
    assign_pci_mmio_dma_irq_capabilities(capabilities, idx, 2, 0xfed6_0000, 0x20_000, 51);
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
                Some(StartupState::Failed) | None => return DependencyStatus::Failed,
                Some(StartupState::Pending) | Some(StartupState::Starting) => {
                    return DependencyStatus::Waiting;
                }
            }
        }
        dep_idx += 1;
    }
    DependencyStatus::Ready(parent)
}

/// Supervisor entry point for early service lifecycle management.
#[derive(Clone, Copy, Debug, Default)]
pub struct Supervisor;

impl Supervisor {
    pub const fn new() -> Self {
        Self
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
                    if record.state == StartupState::Pending {
                        match dependencies_ready(record.descriptor, &report) {
                            DependencyStatus::Ready(parent) => {
                                if !service_manifest_signature_valid(record.descriptor) {
                                    report.set_failed(
                                        idx,
                                        KernelError::SecurityViolation(
                                            IsolationError::PolicyViolation,
                                        ),
                                    );
                                    made_progress = true;
                                    idx += 1;
                                    continue;
                                }

                                report.set_starting(idx);
                                let request = SpawnTaskRequest {
                                    parent,
                                    entry_point: record.descriptor.entry_point,
                                    priority: record.descriptor.priority,
                                    credentials: record.descriptor.credentials,
                                };

                                match kernel.spawn_task(request) {
                                    Ok(pid) => {
                                        if let Err(error) =
                                            reset_spawned_service_capabilities(kernel, pid)
                                        {
                                            report.set_failed(idx, error);
                                            made_progress = true;
                                            idx += 1;
                                            continue;
                                        }
                                        report.set_running(idx, pid);
                                        if let Err(error) =
                                            report.regrant_record_capabilities(kernel, idx, pid)
                                        {
                                            report.set_failed(idx, error);
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
                                    Err(error) => report.set_failed(idx, error),
                                }
                                made_progress = true;
                            }
                            DependencyStatus::Waiting => {
                                pending += 1;
                            }
                            DependencyStatus::Failed => {
                                report.set_failed(idx, KernelError::InvalidArgument);
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
                        if record.state == StartupState::Pending {
                            report.set_failed(fail_idx, KernelError::InvalidArgument);
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
                base: 0xfed0_0000,
                length: 0x20_000,
            },
        )
        .and_then(|capability| capability.id)
        .unwrap();
        let old_mmio = assigned_capability(
            &report,
            ServiceId::Nvmed,
            CapabilityObject::MemoryObject(0xfed0_0000),
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
                base: 0xfed0_0000,
                length: 0x20_000,
            },
        )
        .and_then(|capability| capability.id)
        .unwrap();
        let new_mmio = assigned_capability(
            &report,
            ServiceId::Nvmed,
            CapabilityObject::MemoryObject(0xfed0_0000),
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
        assert_eq!(report.generation(ServiceId::Displayd), Some(1));
        assert_eq!(
            kernel.service_owner(RegistryServiceId::Displayd),
            Some(new_displayd)
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
