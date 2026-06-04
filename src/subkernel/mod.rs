//! The Mirage L2 security kernel responsible for authentication and isolation.

use crate::kernel::memory::MemoryProtection;
use crate::kernel::process::{ExecRequest, ProcessId, MAX_SUPPLEMENTARY_GROUPS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityLevel {
    Public = 0,
    Internal = 1,
    Confidential = 2,
    System = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecurityLabel {
    level: SecurityLevel,
    categories: u32,
}

impl SecurityLabel {
    pub const fn new(level: SecurityLevel, categories: u32) -> Self {
        Self { level, categories }
    }

    pub const fn public() -> Self {
        Self::new(SecurityLevel::Public, 0)
    }

    pub const fn internal() -> Self {
        Self::new(SecurityLevel::Internal, 0)
    }

    pub const fn confidential() -> Self {
        Self::new(SecurityLevel::Confidential, 0)
    }

    pub const fn system() -> Self {
        Self::new(SecurityLevel::System, u32::MAX)
    }

    pub const fn level(&self) -> SecurityLevel {
        self.level
    }

    pub const fn categories(&self) -> u32 {
        self.categories
    }

    pub fn dominates(&self, other: &SecurityLabel) -> bool {
        (self.level as u8) >= (other.level as u8)
            && (self.categories & other.categories) == other.categories
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityClass {
    Public,
    Internal,
    Confidential,
    System,
}

impl SecurityClass {
    pub const fn as_label(self) -> SecurityLabel {
        match self {
            SecurityClass::Public => SecurityLabel::public(),
            SecurityClass::Internal => SecurityLabel::internal(),
            SecurityClass::Confidential => SecurityLabel::confidential(),
            SecurityClass::System => SecurityLabel::system(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsolationLevel {
    None,
    Process,
    VirtualMachine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityId(u64);

impl CapabilityId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    pub const fn raw(&self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityObject {
    IpcEndpoint(ProcessId),
    IrqLine(u16),
    DmaRegion { base: u64, length: u64 },
    PciDevice(u64),
    MmioRegion { base: u64, length: u64 },
    VramRegion { base: u64, length: u64 },
    Framebuffer { base: u64, length: u64 },
    HotplugController(u64),
    BlockDeviceRegistry,
    DisplayRegistry,
    FsObject(u64),
    ServiceControl,
    ModuleLoad,
    ProcessHandle(ProcessId),
    MemoryObject(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityRight {
    Read,
    Write,
    Send,
    Receive,
    Control,
    Transfer,
    Revoke,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityRights {
    flags: u16,
}

const RIGHT_READ: u16 = 0b0000001;
const RIGHT_WRITE: u16 = 0b0000010;
const RIGHT_SEND: u16 = 0b0000100;
const RIGHT_RECEIVE: u16 = 0b0001000;
const RIGHT_CONTROL: u16 = 0b0010000;
const RIGHT_TRANSFER: u16 = 0b0100000;
const RIGHT_REVOKE: u16 = 0b1000000;

impl CapabilityRights {
    pub const fn new(flags: u16) -> Self {
        Self { flags }
    }

    pub const fn none() -> Self {
        Self::new(0)
    }

    pub const fn all() -> Self {
        Self::new(
            RIGHT_READ
                | RIGHT_WRITE
                | RIGHT_SEND
                | RIGHT_RECEIVE
                | RIGHT_CONTROL
                | RIGHT_TRANSFER
                | RIGHT_REVOKE,
        )
    }

    pub const fn io() -> Self {
        Self::new(RIGHT_READ | RIGHT_WRITE | RIGHT_CONTROL | RIGHT_TRANSFER | RIGHT_REVOKE)
    }

    pub const fn ipc_endpoint() -> Self {
        Self::new(RIGHT_SEND | RIGHT_RECEIVE | RIGHT_TRANSFER | RIGHT_REVOKE)
    }

    pub const fn service_control() -> Self {
        Self::new(RIGHT_CONTROL | RIGHT_TRANSFER | RIGHT_REVOKE)
    }

    pub const fn memory() -> Self {
        Self::new(RIGHT_READ | RIGHT_WRITE | RIGHT_CONTROL | RIGHT_TRANSFER | RIGHT_REVOKE)
    }

    pub const fn process_control() -> Self {
        Self::new(RIGHT_CONTROL | RIGHT_TRANSFER | RIGHT_REVOKE)
    }

    pub const fn with(mut self, right: CapabilityRight) -> Self {
        self.flags |= right.flag();
        self
    }

    pub const fn without(mut self, right: CapabilityRight) -> Self {
        self.flags &= !right.flag();
        self
    }

    pub const fn contains(self, right: CapabilityRight) -> bool {
        (self.flags & right.flag()) != 0
    }

    pub const fn contains_all(self, requested: CapabilityRights) -> bool {
        (self.flags & requested.flags) == requested.flags
    }

    pub const fn inherited_child(self) -> Self {
        self.without(CapabilityRight::Revoke)
    }

    pub const fn raw(&self) -> u16 {
        self.flags
    }
}

impl CapabilityRight {
    const fn flag(self) -> u16 {
        match self {
            CapabilityRight::Read => RIGHT_READ,
            CapabilityRight::Write => RIGHT_WRITE,
            CapabilityRight::Send => RIGHT_SEND,
            CapabilityRight::Receive => RIGHT_RECEIVE,
            CapabilityRight::Control => RIGHT_CONTROL,
            CapabilityRight::Transfer => RIGHT_TRANSFER,
            CapabilityRight::Revoke => RIGHT_REVOKE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityRecord {
    id: CapabilityId,
    owner: ProcessId,
    object: CapabilityObject,
    rights: CapabilityRights,
    parent: Option<CapabilityId>,
}

impl CapabilityRecord {
    pub const fn new(
        id: CapabilityId,
        owner: ProcessId,
        object: CapabilityObject,
        rights: CapabilityRights,
        parent: Option<CapabilityId>,
    ) -> Self {
        Self {
            id,
            owner,
            object,
            rights,
            parent,
        }
    }

    pub const fn id(&self) -> CapabilityId {
        self.id
    }

    pub const fn owner(&self) -> ProcessId {
        self.owner
    }

    pub const fn object(&self) -> CapabilityObject {
        self.object
    }

    pub const fn rights(&self) -> CapabilityRights {
        self.rights
    }

    pub const fn parent(&self) -> Option<CapabilityId> {
        self.parent
    }
}

pub const MAX_CAPABILITY_RECORDS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilitySet {
    flags: u32,
}

pub const CAP_IPC: u32 = 0b0001;
pub const CAP_SPAWN: u32 = 0b0010;
pub const CAP_KERNEL: u32 = 0b0100;
pub const CAP_IO: u32 = 0b1000;

impl CapabilitySet {
    pub const fn new(flags: u32) -> Self {
        Self { flags }
    }

    pub const fn none() -> Self {
        Self::new(0)
    }

    pub const fn full() -> Self {
        Self::new(CAP_IPC | CAP_SPAWN | CAP_KERNEL | CAP_IO)
    }

    pub const fn ipc() -> Self {
        Self::new(CAP_IPC)
    }

    pub const fn ipc_io() -> Self {
        Self::new(CAP_IPC | CAP_IO)
    }

    pub fn allows_ipc(&self) -> bool {
        (self.flags & CAP_IPC) != 0
    }

    pub fn allows_spawn(&self) -> bool {
        (self.flags & CAP_SPAWN) != 0
    }

    pub fn allows_kernel_access(&self) -> bool {
        (self.flags & CAP_KERNEL) != 0
    }

    pub fn allows_io(&self) -> bool {
        (self.flags & CAP_IO) != 0
    }

    pub fn contains(&self, requested: CapabilitySet) -> bool {
        (self.flags & requested.flags) == requested.flags
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Credentials {
    label: SecurityLabel,
    capabilities: CapabilitySet,
    isolation: IsolationLevel,
    uid: u16,
    euid: u16,
    gid: u16,
    egid: u16,
    supplementary_groups: [u16; MAX_SUPPLEMENTARY_GROUPS],
    supplementary_group_count: usize,
}

impl Credentials {
    pub const fn new(
        label: SecurityLabel,
        capabilities: CapabilitySet,
        isolation: IsolationLevel,
    ) -> Self {
        let uid = match label.level() {
            SecurityLevel::System => 0,
            _ => 1000,
        };
        Self::with_unix_credentials(
            label,
            capabilities,
            isolation,
            uid,
            uid,
            uid,
            uid,
            [0; MAX_SUPPLEMENTARY_GROUPS],
            0,
        )
    }

    pub const fn with_unix_credentials(
        label: SecurityLabel,
        capabilities: CapabilitySet,
        isolation: IsolationLevel,
        uid: u16,
        euid: u16,
        gid: u16,
        egid: u16,
        supplementary_groups: [u16; MAX_SUPPLEMENTARY_GROUPS],
        supplementary_group_count: usize,
    ) -> Self {
        Self {
            label,
            capabilities,
            isolation,
            uid,
            euid,
            gid,
            egid,
            supplementary_groups,
            supplementary_group_count,
        }
    }

    pub const fn system() -> Self {
        Self::with_unix_credentials(
            SecurityLabel::system(),
            CapabilitySet::full(),
            IsolationLevel::Process,
            0,
            0,
            0,
            0,
            [0; MAX_SUPPLEMENTARY_GROUPS],
            0,
        )
    }

    pub const fn user() -> Self {
        Self::with_unix_credentials(
            SecurityLabel::internal(),
            CapabilitySet::ipc(),
            IsolationLevel::None,
            1000,
            1000,
            1000,
            1000,
            [0; MAX_SUPPLEMENTARY_GROUPS],
            0,
        )
    }

    pub const fn label(&self) -> SecurityLabel {
        self.label
    }

    pub const fn capabilities(&self) -> CapabilitySet {
        self.capabilities
    }

    pub const fn isolation(&self) -> IsolationLevel {
        self.isolation
    }

    pub const fn uid(&self) -> u16 {
        self.uid
    }

    pub const fn euid(&self) -> u16 {
        self.euid
    }

    pub const fn gid(&self) -> u16 {
        self.gid
    }

    pub const fn egid(&self) -> u16 {
        self.egid
    }

    pub const fn supplementary_groups(&self) -> [u16; MAX_SUPPLEMENTARY_GROUPS] {
        self.supplementary_groups
    }

    pub const fn supplementary_group_count(&self) -> usize {
        self.supplementary_group_count
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TaskDomain {
    pid: ProcessId,
    label: SecurityLabel,
    capabilities: CapabilitySet,
    isolation: IsolationLevel,
    uid: u16,
    euid: u16,
    gid: u16,
    egid: u16,
    supplementary_groups: [u16; MAX_SUPPLEMENTARY_GROUPS],
    supplementary_group_count: usize,
    quarantine_events: u32,
}

impl TaskDomain {
    pub const fn from_credentials(pid: ProcessId, creds: Credentials) -> Self {
        Self {
            pid,
            label: creds.label(),
            capabilities: creds.capabilities(),
            isolation: creds.isolation(),
            uid: creds.uid(),
            euid: creds.euid(),
            gid: creds.gid(),
            egid: creds.egid(),
            supplementary_groups: creds.supplementary_groups(),
            supplementary_group_count: creds.supplementary_group_count(),
            quarantine_events: 0,
        }
    }

    pub fn can_transmit(&self, class: SecurityClass) -> bool {
        self.capabilities.allows_ipc() && self.label.dominates(&class.as_label())
    }

    pub fn can_receive(&self, class: SecurityClass) -> bool {
        self.label.dominates(&class.as_label())
    }

    fn has_system_privilege(&self) -> bool {
        self.label.level() == SecurityLevel::System || self.capabilities.allows_kernel_access()
    }

    fn can_delegate(&self, requested: Credentials) -> bool {
        self.label.dominates(&requested.label())
            && self.capabilities.contains(requested.capabilities())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceSecurity {
    class: SecurityClass,
    requires_kernel_mode: bool,
}

impl DeviceSecurity {
    pub const fn new(class: SecurityClass, requires_kernel_mode: bool) -> Self {
        Self {
            class,
            requires_kernel_mode,
        }
    }

    pub const fn class(&self) -> SecurityClass {
        self.class
    }

    pub const fn requires_kernel_mode(&self) -> bool {
        self.requires_kernel_mode
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsolationError {
    UnknownTask,
    PolicyViolation,
    CapabilityMissing,
    CapabilityTableFull,
}

#[derive(Clone, Copy)]
pub struct SecurityKernel<const MAX: usize> {
    domains: [Option<TaskDomain>; MAX],
    capabilities: [Option<CapabilityRecord>; MAX_CAPABILITY_RECORDS],
    next_capability_id: u64,
}

impl<const MAX: usize> SecurityKernel<MAX> {
    pub const fn new() -> Self {
        Self {
            domains: [None; MAX],
            capabilities: [None; MAX_CAPABILITY_RECORDS],
            next_capability_id: 1,
        }
    }

    pub fn reset(&mut self) {
        let mut idx = 0;
        while idx < MAX {
            self.domains[idx] = None;
            idx += 1;
        }

        idx = 0;
        while idx < MAX_CAPABILITY_RECORDS {
            self.capabilities[idx] = None;
            idx += 1;
        }
        self.next_capability_id = 1;
    }

    pub fn register_task(
        &mut self,
        pid: ProcessId,
        creds: Credentials,
    ) -> Result<(), IsolationError> {
        if let Some(idx) = self.find_domain_index(pid) {
            let previous = self.domains[idx];
            self.domains[idx] = Some(TaskDomain::from_credentials(pid, creds));
            self.revoke_all_capabilities(pid);
            if let Err(err) = self.seed_initial_capabilities(pid, creds) {
                self.revoke_all_capabilities(pid);
                self.domains[idx] = previous;
                return Err(err);
            }
            return Ok(());
        }

        let mut idx = 0;
        while idx < MAX {
            if self.domains[idx].is_none() {
                self.domains[idx] = Some(TaskDomain::from_credentials(pid, creds));
                if let Err(err) = self.seed_initial_capabilities(pid, creds) {
                    self.domains[idx] = None;
                    return Err(err);
                }
                return Ok(());
            }
            idx += 1;
        }

        Err(IsolationError::PolicyViolation)
    }

    pub fn revoke_task(&mut self, pid: ProcessId) {
        if let Some(idx) = self.find_domain_index(pid) {
            self.domains[idx] = None;
        }
        self.revoke_all_capabilities(pid);
    }

    pub fn grant_capability(
        &mut self,
        owner: ProcessId,
        object: CapabilityObject,
        rights: CapabilityRights,
    ) -> Result<CapabilityId, IsolationError> {
        self.domain(owner)?;
        self.insert_capability(owner, object, rights, None)
    }

    pub fn revoke_capability(&mut self, id: CapabilityId) -> Result<(), IsolationError> {
        let idx = self
            .find_capability_index(id)
            .ok_or(IsolationError::CapabilityMissing)?;
        self.capabilities[idx] = None;

        let mut child_idx = 0;
        while child_idx < MAX_CAPABILITY_RECORDS {
            if let Some(record) = self.capabilities[child_idx] {
                if record.parent == Some(id) {
                    self.capabilities[child_idx] = None;
                }
            }
            child_idx += 1;
        }

        Ok(())
    }

    pub fn check_capability(
        &self,
        owner: ProcessId,
        object: CapabilityObject,
        right: CapabilityRight,
    ) -> Result<(), IsolationError> {
        self.domain(owner)?;
        if self.has_capability(owner, object, right) {
            Ok(())
        } else {
            Err(IsolationError::CapabilityMissing)
        }
    }

    pub fn transfer_capability(
        &mut self,
        source: ProcessId,
        target: ProcessId,
        id: CapabilityId,
    ) -> Result<CapabilityId, IsolationError> {
        self.domain(source)?;
        self.domain(target)?;
        let idx = self
            .find_capability_index(id)
            .ok_or(IsolationError::CapabilityMissing)?;
        let record = self.capabilities[idx].ok_or(IsolationError::CapabilityMissing)?;

        if record.owner != source || !record.rights.contains(CapabilityRight::Transfer) {
            return Err(IsolationError::CapabilityMissing);
        }

        self.insert_capability(
            target,
            record.object,
            record.rights.inherited_child(),
            Some(id),
        )
    }

    pub fn derive_inherited_child_capabilities(
        &mut self,
        parent: ProcessId,
        child: ProcessId,
    ) -> Result<(), IsolationError> {
        self.domain(parent)?;
        self.domain(child)?;

        let snapshot = self.capabilities;
        let mut idx = 0;
        while idx < MAX_CAPABILITY_RECORDS {
            if let Some(record) = snapshot[idx] {
                if record.owner == parent && record.rights.contains(CapabilityRight::Transfer) {
                    self.insert_capability(
                        child,
                        record.object,
                        record.rights.inherited_child(),
                        Some(record.id),
                    )?;
                }
            }
            idx += 1;
        }

        Ok(())
    }

    pub fn revoke_all_capabilities(&mut self, owner: ProcessId) {
        let mut idx = 0;
        while idx < MAX_CAPABILITY_RECORDS {
            if let Some(record) = self.capabilities[idx] {
                if record.owner == owner {
                    self.capabilities[idx] = None;
                }
            }
            idx += 1;
        }
    }

    pub fn authorize_ipc(
        &self,
        sender: ProcessId,
        receiver: ProcessId,
        class: SecurityClass,
    ) -> Result<(), IsolationError> {
        let sender_domain = self.domain(sender)?;
        let receiver_domain = self.domain(receiver)?;

        self.check_capability(
            sender,
            CapabilityObject::IpcEndpoint(receiver),
            CapabilityRight::Send,
        )?;

        if !sender_domain.can_transmit(class) || !receiver_domain.can_receive(class) {
            return Err(IsolationError::PolicyViolation);
        }

        if sender_domain.isolation == IsolationLevel::VirtualMachine
            && receiver_domain.isolation == IsolationLevel::None
        {
            return Err(IsolationError::PolicyViolation);
        }

        Ok(())
    }

    pub fn authorize_device_access(
        &self,
        pid: ProcessId,
        object: CapabilityObject,
        required_right: CapabilityRight,
        security: DeviceSecurity,
    ) -> Result<(), IsolationError> {
        let domain = self.domain(pid)?;

        self.check_capability(pid, object, required_right)?;

        if security.requires_kernel_mode() {
            self.check_capability(pid, CapabilityObject::ModuleLoad, CapabilityRight::Control)?;
        }

        if !domain.label.dominates(&security.class().as_label()) {
            return Err(IsolationError::PolicyViolation);
        }

        Ok(())
    }

    pub fn authorize_ipc_receive(&self, pid: ProcessId) -> Result<(), IsolationError> {
        self.domain(pid)?;
        self.check_capability(
            pid,
            CapabilityObject::IpcEndpoint(pid),
            CapabilityRight::Receive,
        )
    }

    /// Authorize the L2/subkernel control plane to bind a service endpoint.
    pub fn authorize_service_control(&self, pid: ProcessId) -> Result<(), IsolationError> {
        self.domain(pid)?;
        self.check_capability(
            pid,
            CapabilityObject::ServiceControl,
            CapabilityRight::Control,
        )
    }

    /// Authorize changes to mutable Unix credential state (uid/gid/groups).
    pub fn authorize_credential_update(&self, pid: ProcessId) -> Result<(), IsolationError> {
        self.domain(pid)?;
        self.check_capability(
            pid,
            CapabilityObject::ProcessHandle(pid),
            CapabilityRight::Control,
        )
    }

    /// Authorize a task domain to own a service advertised at the given class.
    pub fn authorize_service_registration(
        &self,
        pid: ProcessId,
        class: SecurityClass,
    ) -> Result<(), IsolationError> {
        let domain = self.domain(pid)?;
        self.check_capability(
            pid,
            CapabilityObject::IpcEndpoint(pid),
            CapabilityRight::Receive,
        )?;
        if domain.can_receive(class) {
            Ok(())
        } else {
            Err(IsolationError::PolicyViolation)
        }
    }

    pub fn authorize_spawn(
        &self,
        parent: ProcessId,
        requested: Credentials,
    ) -> Result<(), IsolationError> {
        let parent_domain = self.domain(parent)?;

        if !parent_domain.capabilities.allows_spawn() {
            return Err(IsolationError::CapabilityMissing);
        }

        if !parent_domain.has_system_privilege() && !parent_domain.can_delegate(requested) {
            return Err(IsolationError::PolicyViolation);
        }

        Ok(())
    }

    /// Authorize an exec image replacement. Unlike spawn, exec does not require
    /// CAP_SPAWN because the process is not creating a new task domain. L2 still
    /// verifies filesystem executable metadata and rejects credential escalation
    /// unless the target is modeled as a signed service daemon.
    pub fn authorize_exec(&self, request: &ExecRequest) -> Result<(), IsolationError> {
        let domain = self.domain(request.caller)?;

        if !request.image.is_executable() {
            return Err(IsolationError::PolicyViolation);
        }

        if request.argv.truncated || request.envp.truncated {
            return Err(IsolationError::PolicyViolation);
        }

        let requested = request.requested_credentials;
        if domain.has_system_privilege() || domain.can_delegate(requested) {
            return Ok(());
        }

        if request.image.is_signed_service_daemon() {
            return Ok(());
        }

        Err(IsolationError::PolicyViolation)
    }

    pub fn authorize_device_enumeration(&self, pid: ProcessId) -> Result<(), IsolationError> {
        self.domain(pid)?;
        self.check_capability(
            pid,
            CapabilityObject::PciDevice(u64::MAX),
            CapabilityRight::Read,
        )
    }

    pub fn authorize_memory_service(&self, pid: ProcessId) -> Result<(), IsolationError> {
        self.enforce_isolation(pid)
    }

    pub fn authorize_memory_mapping(
        &self,
        pid: ProcessId,
        protection: MemoryProtection,
    ) -> Result<(), IsolationError> {
        let domain = self.domain(pid)?;

        if protection.write && protection.execute {
            return Err(IsolationError::PolicyViolation);
        }

        if protection.execute && !domain.has_system_privilege() {
            return Err(IsolationError::CapabilityMissing);
        }

        self.enforce_isolation(pid)
    }

    pub fn credentials(&self, pid: ProcessId) -> Result<Credentials, IsolationError> {
        let domain = self.domain(pid)?;
        Ok(Credentials::with_unix_credentials(
            domain.label,
            domain.capabilities,
            domain.isolation,
            domain.uid,
            domain.euid,
            domain.gid,
            domain.egid,
            domain.supplementary_groups,
            domain.supplementary_group_count,
        ))
    }

    pub fn enforce_isolation(&self, pid: ProcessId) -> Result<(), IsolationError> {
        let domain = self.domain(pid)?;
        match domain.isolation {
            IsolationLevel::None => Ok(()),
            IsolationLevel::Process => Ok(()),
            IsolationLevel::VirtualMachine => {
                if domain.quarantine_events > 0 {
                    Err(IsolationError::PolicyViolation)
                } else {
                    Ok(())
                }
            }
        }
    }

    fn seed_initial_capabilities(
        &mut self,
        pid: ProcessId,
        creds: Credentials,
    ) -> Result<(), IsolationError> {
        let caps = creds.capabilities();

        if caps.allows_ipc() {
            self.insert_capability(
                pid,
                CapabilityObject::IpcEndpoint(pid),
                CapabilityRights::ipc_endpoint(),
                None,
            )?;
            self.insert_capability(
                pid,
                CapabilityObject::IpcEndpoint(ProcessId::new(u64::MAX)),
                CapabilityRights::ipc_endpoint(),
                None,
            )?;
        }

        if caps.allows_io() {
            self.insert_capability(
                pid,
                CapabilityObject::PciDevice(u64::MAX),
                CapabilityRights::io(),
                None,
            )?;
            self.insert_capability(
                pid,
                CapabilityObject::DmaRegion {
                    base: 0,
                    length: u64::MAX,
                },
                CapabilityRights::io(),
                None,
            )?;
            self.insert_capability(
                pid,
                CapabilityObject::IrqLine(u16::MAX),
                CapabilityRights::io(),
                None,
            )?;
            self.insert_capability(
                pid,
                CapabilityObject::MmioRegion {
                    base: 0,
                    length: u64::MAX,
                },
                CapabilityRights::io(),
                None,
            )?;
        }

        if caps.allows_kernel_access() || creds.label().level() == SecurityLevel::System {
            self.insert_capability(
                pid,
                CapabilityObject::ServiceControl,
                CapabilityRights::service_control(),
                None,
            )?;
            self.insert_capability(
                pid,
                CapabilityObject::ModuleLoad,
                CapabilityRights::service_control(),
                None,
            )?;
            self.insert_capability(
                pid,
                CapabilityObject::ProcessHandle(pid),
                CapabilityRights::process_control(),
                None,
            )?;
            self.insert_capability(
                pid,
                CapabilityObject::ProcessHandle(ProcessId::new(u64::MAX)),
                CapabilityRights::process_control(),
                None,
            )?;
            self.insert_capability(
                pid,
                CapabilityObject::MemoryObject(u64::MAX),
                CapabilityRights::memory(),
                None,
            )?;
        }

        Ok(())
    }

    fn insert_capability(
        &mut self,
        owner: ProcessId,
        object: CapabilityObject,
        rights: CapabilityRights,
        parent: Option<CapabilityId>,
    ) -> Result<CapabilityId, IsolationError> {
        let mut idx = 0;
        while idx < MAX_CAPABILITY_RECORDS {
            if self.capabilities[idx].is_none() {
                let id = CapabilityId::new(self.next_capability_id);
                self.next_capability_id = self.next_capability_id.wrapping_add(1);
                if self.next_capability_id == 0 {
                    self.next_capability_id = 1;
                }
                self.capabilities[idx] =
                    Some(CapabilityRecord::new(id, owner, object, rights, parent));
                return Ok(id);
            }
            idx += 1;
        }

        Err(IsolationError::CapabilityTableFull)
    }

    fn has_capability(
        &self,
        owner: ProcessId,
        object: CapabilityObject,
        right: CapabilityRight,
    ) -> bool {
        let mut idx = 0;
        while idx < MAX_CAPABILITY_RECORDS {
            if let Some(record) = self.capabilities[idx] {
                if record.owner == owner
                    && Self::capability_object_matches(record.object, object)
                    && record.rights.contains(right)
                {
                    return true;
                }
            }
            idx += 1;
        }
        false
    }

    fn capability_object_matches(granted: CapabilityObject, requested: CapabilityObject) -> bool {
        if granted == requested {
            return true;
        }

        match (granted, requested) {
            (CapabilityObject::IpcEndpoint(pid), CapabilityObject::IpcEndpoint(_))
            | (CapabilityObject::ProcessHandle(pid), CapabilityObject::ProcessHandle(_)) => {
                pid.raw() == u64::MAX
            }
            (CapabilityObject::PciDevice(id), CapabilityObject::PciDevice(_))
            | (CapabilityObject::FsObject(id), CapabilityObject::FsObject(_))
            | (CapabilityObject::HotplugController(id), CapabilityObject::HotplugController(_))
            | (CapabilityObject::MemoryObject(id), CapabilityObject::MemoryObject(_)) => {
                id == u64::MAX
            }
            (CapabilityObject::IrqLine(line), CapabilityObject::IrqLine(_)) => line == u16::MAX,
            (
                CapabilityObject::DmaRegion {
                    base: granted_base,
                    length: granted_length,
                }
                | CapabilityObject::MmioRegion {
                    base: granted_base,
                    length: granted_length,
                }
                | CapabilityObject::VramRegion {
                    base: granted_base,
                    length: granted_length,
                }
                | CapabilityObject::Framebuffer {
                    base: granted_base,
                    length: granted_length,
                },
                CapabilityObject::DmaRegion {
                    base: requested_base,
                    length: requested_length,
                }
                | CapabilityObject::MmioRegion {
                    base: requested_base,
                    length: requested_length,
                }
                | CapabilityObject::VramRegion {
                    base: requested_base,
                    length: requested_length,
                }
                | CapabilityObject::Framebuffer {
                    base: requested_base,
                    length: requested_length,
                },
            ) => {
                let granted_end = granted_base.saturating_add(granted_length);
                let requested_end = requested_base.saturating_add(requested_length);
                requested_base >= granted_base && requested_end <= granted_end
            }
            _ => false,
        }
    }

    fn find_capability_index(&self, id: CapabilityId) -> Option<usize> {
        let mut idx = 0;
        while idx < MAX_CAPABILITY_RECORDS {
            if let Some(record) = self.capabilities[idx] {
                if record.id == id {
                    return Some(idx);
                }
            }
            idx += 1;
        }
        None
    }

    fn domain(&self, pid: ProcessId) -> Result<TaskDomain, IsolationError> {
        self.find_domain_index(pid)
            .and_then(|idx| self.domains[idx])
            .ok_or(IsolationError::UnknownTask)
    }

    fn find_domain_index(&self, pid: ProcessId) -> Option<usize> {
        let mut idx = 0;
        while idx < MAX {
            if let Some(domain) = self.domains[idx] {
                if domain.pid == pid {
                    return Some(idx);
                }
            }
            idx += 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(raw: u64) -> ProcessId {
        ProcessId::new(raw)
    }

    fn exec_request(
        caller: ProcessId,
        requested_credentials: Credentials,
        mode: u16,
        signature: Option<crate::kernel::process::ExecSignatureMetadata>,
    ) -> crate::kernel::process::ExecRequest {
        let path = crate::kernel::fs::Path::new("/bin/app").unwrap();
        crate::kernel::process::ExecRequest::new(
            caller,
            crate::kernel::process::ProcessPath::from_path(path),
            crate::kernel::process::ExecVectorMetadata::empty(),
            crate::kernel::process::ExecVectorMetadata::empty(),
            requested_credentials,
            crate::kernel::process::ExecImageMetadata::new(
                7,
                4096,
                mode,
                0x1000,
                0x8000,
                0x9000,
                signature.map(|_| crate::kernel::process::ExecServiceDaemon::Display),
                signature,
            ),
        )
    }

    #[test]
    fn authorize_exec_allows_same_credentials_without_spawn_capability() {
        let mut security: SecurityKernel<4> = SecurityKernel::new();
        security.register_task(pid(1), Credentials::user()).unwrap();

        let request = exec_request(pid(1), Credentials::user(), 0o755, None);

        assert_eq!(security.authorize_exec(&request), Ok(()));
    }

    #[test]
    fn authorize_exec_rejects_unsigned_escalation_and_non_executable_image() {
        let mut security: SecurityKernel<4> = SecurityKernel::new();
        security.register_task(pid(1), Credentials::user()).unwrap();

        let elevated = Credentials::new(
            SecurityLabel::internal(),
            CapabilitySet::ipc_io(),
            IsolationLevel::Process,
        );
        let unsigned = exec_request(pid(1), elevated, 0o755, None);
        assert_eq!(
            security.authorize_exec(&unsigned),
            Err(IsolationError::PolicyViolation)
        );

        let non_executable = exec_request(pid(1), Credentials::user(), 0o644, None);
        assert_eq!(
            security.authorize_exec(&non_executable),
            Err(IsolationError::PolicyViolation)
        );
    }

    #[test]
    fn authorize_exec_allows_signed_service_daemon_escalation() {
        let mut security: SecurityKernel<4> = SecurityKernel::new();
        security.register_task(pid(1), Credentials::user()).unwrap();

        let elevated = Credentials::new(
            SecurityLabel::internal(),
            CapabilitySet::ipc_io(),
            IsolationLevel::Process,
        );
        let signed = exec_request(
            pid(1),
            elevated,
            0o755,
            Some(crate::kernel::process::ExecSignatureMetadata::new(
                "mirage-service-root",
                0x444953504c415944,
            )),
        );

        assert_eq!(security.authorize_exec(&signed), Ok(()));
    }

    #[test]
    fn capability_table_grants_revokes_and_checks_object_rights() {
        let mut security: SecurityKernel<4> = SecurityKernel::new();
        security.register_task(pid(1), Credentials::user()).unwrap();

        let object = CapabilityObject::PciDevice(7);
        let cap = security
            .grant_capability(pid(1), object, CapabilityRights::io())
            .unwrap();

        assert_eq!(
            security.check_capability(pid(1), object, CapabilityRight::Read),
            Ok(())
        );
        assert_eq!(
            security.check_capability(
                pid(1),
                CapabilityObject::PciDevice(8),
                CapabilityRight::Read
            ),
            Err(IsolationError::CapabilityMissing)
        );

        security.revoke_capability(cap).unwrap();
        assert_eq!(
            security.check_capability(pid(1), object, CapabilityRight::Read),
            Err(IsolationError::CapabilityMissing)
        );
    }

    #[test]
    fn capability_transfer_and_child_inheritance_use_transferable_rights() {
        let mut security: SecurityKernel<4> = SecurityKernel::new();
        security.register_task(pid(1), Credentials::user()).unwrap();
        security.register_task(pid(2), Credentials::user()).unwrap();
        security.register_task(pid(3), Credentials::user()).unwrap();

        let object = CapabilityObject::MemoryObject(42);
        let cap = security
            .grant_capability(pid(1), object, CapabilityRights::memory())
            .unwrap();
        let child_cap = security.transfer_capability(pid(1), pid(2), cap).unwrap();

        assert_eq!(
            security.check_capability(pid(2), object, CapabilityRight::Control),
            Ok(())
        );
        assert_eq!(
            security.check_capability(pid(2), object, CapabilityRight::Revoke),
            Err(IsolationError::CapabilityMissing)
        );

        security
            .derive_inherited_child_capabilities(pid(1), pid(3))
            .unwrap();
        assert_eq!(
            security.check_capability(pid(3), object, CapabilityRight::Control),
            Ok(())
        );

        security.revoke_capability(cap).unwrap();
        assert_eq!(
            security.revoke_capability(child_cap),
            Err(IsolationError::CapabilityMissing)
        );
    }

    #[test]
    fn device_authorization_requires_scoped_device_capability() {
        let mut security: SecurityKernel<4> = SecurityKernel::new();
        security.register_task(pid(1), Credentials::user()).unwrap();
        let security_class = DeviceSecurity::new(SecurityClass::Internal, false);

        assert_eq!(
            security.authorize_device_access(
                pid(1),
                CapabilityObject::PciDevice(3),
                CapabilityRight::Read,
                security_class,
            ),
            Err(IsolationError::CapabilityMissing)
        );

        security
            .grant_capability(
                pid(1),
                CapabilityObject::PciDevice(3),
                CapabilityRights::io(),
            )
            .unwrap();
        assert_eq!(
            security.authorize_device_access(
                pid(1),
                CapabilityObject::PciDevice(3),
                CapabilityRight::Read,
                security_class,
            ),
            Ok(())
        );
        assert_eq!(
            security.authorize_device_access(
                pid(1),
                CapabilityObject::PciDevice(4),
                CapabilityRight::Read,
                security_class,
            ),
            Err(IsolationError::CapabilityMissing)
        );
    }

    #[test]
    fn authorize_spawn_requires_spawn_capability() {
        let mut security: SecurityKernel<4> = SecurityKernel::new();
        security.register_task(pid(1), Credentials::user()).unwrap();

        assert_eq!(
            security.authorize_spawn(pid(1), Credentials::user()),
            Err(IsolationError::CapabilityMissing)
        );
    }

    #[test]
    fn authorize_spawn_allows_delegated_subset() {
        let mut security: SecurityKernel<4> = SecurityKernel::new();
        let parent_creds = Credentials::new(
            SecurityLabel::internal(),
            CapabilitySet::new(CAP_IPC | CAP_SPAWN),
            IsolationLevel::Process,
        );
        security.register_task(pid(1), parent_creds).unwrap();

        assert_eq!(
            security.authorize_spawn(pid(1), Credentials::user()),
            Ok(())
        );
    }

    #[test]
    fn authorize_spawn_rejects_label_or_capability_escalation() {
        let mut security: SecurityKernel<4> = SecurityKernel::new();
        let parent_creds = Credentials::new(
            SecurityLabel::internal(),
            CapabilitySet::new(CAP_IPC | CAP_SPAWN),
            IsolationLevel::Process,
        );
        security.register_task(pid(1), parent_creds).unwrap();

        let confidential_child = Credentials::new(
            SecurityLabel::confidential(),
            CapabilitySet::ipc(),
            IsolationLevel::Process,
        );
        assert_eq!(
            security.authorize_spawn(pid(1), confidential_child),
            Err(IsolationError::PolicyViolation)
        );

        let io_child = Credentials::new(
            SecurityLabel::internal(),
            CapabilitySet::new(CAP_IPC | CAP_IO),
            IsolationLevel::Process,
        );
        assert_eq!(
            security.authorize_spawn(pid(1), io_child),
            Err(IsolationError::PolicyViolation)
        );
    }

    #[test]
    fn authorize_spawn_allows_privileged_parent_to_delegate_elevated_credentials() {
        let mut security: SecurityKernel<4> = SecurityKernel::new();
        security
            .register_task(pid(1), Credentials::system())
            .unwrap();

        assert_eq!(
            security.authorize_spawn(pid(1), Credentials::system()),
            Ok(())
        );
    }
}
