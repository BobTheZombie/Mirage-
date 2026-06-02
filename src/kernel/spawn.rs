//! Fixed-capacity L1 startup service supervisor.
//!
//! The supervisor deliberately avoids allocation: service descriptors live in a
//! fixed-size manifest and startup progress is captured in a same-capacity
//! report. This mirrors the kernel process and IPC tables instead of relying on
//! dynamic collections during early boot.

use crate::kernel::process::{
    ExecServiceDaemon, ExecSignatureMetadata, ProcessId, ProcessPriority,
};
use crate::kernel::KernelError;
use crate::subkernel::{CapabilitySet, Credentials, IsolationLevel, SecurityLabel};

/// Number of services in the built-in L1 startup manifest.
pub const MAX_STARTUP_SERVICES: usize = 4;

/// Maximum number of dependencies a startup service can declare.
pub const MAX_SERVICE_DEPENDENCIES: usize = 2;

/// Well-known L1 startup services.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceId {
    L2Subkernel,
    Displayd,
    Networkd,
    Inputd,
}

/// Startup state for a service supervised by L1.
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

/// Per-service startup outcome recorded by the L1 supervisor.
#[derive(Clone, Copy, Debug)]
pub struct ServiceRuntime {
    pub descriptor: ServiceDescriptor,
    pub state: StartupState,
    pub pid: Option<ProcessId>,
    pub failure: Option<KernelError>,
}

impl ServiceRuntime {
    pub const fn pending(descriptor: ServiceDescriptor) -> Self {
        Self {
            descriptor,
            state: StartupState::Pending,
            pid: None,
            failure: None,
        }
    }
}

/// Fixed-capacity startup report produced by `Kernel::spawn_services`.
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
                report.records[idx] = Some(ServiceRuntime::pending(descriptor));
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

    pub(crate) fn set_starting(&mut self, index: usize) {
        if let Some(mut record) = self.records[index] {
            record.state = StartupState::Starting;
            record.failure = None;
            self.records[index] = Some(record);
        }
    }

    pub(crate) fn set_running(&mut self, index: usize, pid: ProcessId) {
        if let Some(mut record) = self.records[index] {
            record.state = StartupState::Running;
            record.pid = Some(pid);
            record.failure = None;
            self.records[index] = Some(record);
        }
    }

    pub(crate) fn set_failed(&mut self, index: usize, error: KernelError) {
        if let Some(mut record) = self.records[index] {
            record.state = StartupState::Failed;
            record.pid = None;
            record.failure = Some(error);
            self.records[index] = Some(record);
        }
    }

    pub(crate) fn dependency_state(&self, service: ServiceId) -> Option<StartupState> {
        self.state(service)
    }

    pub(crate) fn dependency_pid(&self, service: ServiceId) -> Option<ProcessId> {
        self.pid(service)
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
        CapabilitySet::ipc_io(),
        IsolationLevel::Process,
    ),
    [Some(ServiceId::L2Subkernel), None],
    Some(ExecServiceDaemon::Display),
    Some(ExecSignatureMetadata::new(
        "mirage-service-root",
        0x444953504c415944,
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
        CapabilitySet::ipc_io(),
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
        CapabilitySet::ipc_io(),
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
        Some(DISPLAYD_SERVICE),
        Some(NETWORKD_SERVICE),
        Some(INPUTD_SERVICE),
    ],
    MAX_STARTUP_SERVICES,
);

/// Validate static service-daemon signature metadata embedded in the L1 startup
/// manifest. This models the signed-manifest gate for displayd, networkd,
/// inputd, and future L2 driver daemons before they are launched.
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
pub(crate) enum DependencyStatus {
    Ready(Option<ProcessId>),
    Waiting,
    Failed,
}

pub(crate) fn dependencies_ready<const CAP: usize>(
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

pub type DefaultServiceStartupReport = ServiceStartupReport<MAX_STARTUP_SERVICES>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_manifest_blocks_device_daemons_until_l2_runs() {
        let mut report = ServiceStartupReport::from_manifest(&DEFAULT_STARTUP_MANIFEST);
        let l2 = DEFAULT_STARTUP_MANIFEST.descriptor(0).unwrap();
        let displayd = DEFAULT_STARTUP_MANIFEST.descriptor(1).unwrap();

        assert_eq!(
            dependencies_ready(l2, &report),
            DependencyStatus::Ready(None)
        );
        assert_eq!(
            dependencies_ready(displayd, &report),
            DependencyStatus::Waiting
        );

        report.set_running(0, ProcessId::new(1));

        assert_eq!(
            dependencies_ready(displayd, &report),
            DependencyStatus::Ready(Some(ProcessId::new(1)))
        );
    }
}
