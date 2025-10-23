//! The Mirage L2 security kernel responsible for authentication and isolation.

use crate::kernel::process::ProcessId;

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
pub struct CapabilitySet {
    flags: u32,
}

const CAP_IPC: u32 = 0b0001;
const CAP_SPAWN: u32 = 0b0010;
const CAP_KERNEL: u32 = 0b0100;
const CAP_IO: u32 = 0b1000;

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
}

#[derive(Clone, Copy, Debug)]
pub struct Credentials {
    label: SecurityLabel,
    capabilities: CapabilitySet,
    isolation: IsolationLevel,
}

impl Credentials {
    pub const fn new(
        label: SecurityLabel,
        capabilities: CapabilitySet,
        isolation: IsolationLevel,
    ) -> Self {
        Self {
            label,
            capabilities,
            isolation,
        }
    }

    pub const fn system() -> Self {
        Self::new(
            SecurityLabel::system(),
            CapabilitySet::full(),
            IsolationLevel::Process,
        )
    }

    pub const fn user() -> Self {
        Self::new(
            SecurityLabel::internal(),
            CapabilitySet::ipc(),
            IsolationLevel::None,
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
}

#[derive(Clone, Copy, Debug)]
pub struct TaskDomain {
    pid: ProcessId,
    label: SecurityLabel,
    capabilities: CapabilitySet,
    isolation: IsolationLevel,
    quarantine_events: u32,
}

impl TaskDomain {
    pub const fn from_credentials(pid: ProcessId, creds: Credentials) -> Self {
        Self {
            pid,
            label: creds.label(),
            capabilities: creds.capabilities(),
            isolation: creds.isolation(),
            quarantine_events: 0,
        }
    }

    #[inline(always)]
    pub const fn pid(&self) -> ProcessId {
        self.pid
    }

    pub fn can_transmit(&self, class: SecurityClass) -> bool {
        self.capabilities.allows_ipc() && self.label.dominates(&class.as_label())
    }

    pub fn can_receive(&self, class: SecurityClass) -> bool {
        self.label.dominates(&class.as_label())
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
}

/// Open-addressed security domain table that acts as the ultra-low latency
/// bridge between the main kernel and the L2 security core.
#[derive(Clone, Copy)]
pub struct SecurityKernel<const MAX: usize> {
    domains: [Option<TaskDomain>; MAX],
    population: usize,
}

impl<const MAX: usize> SecurityKernel<MAX> {
    pub const fn new() -> Self {
        Self {
            domains: [None; MAX],
            population: 0,
        }
    }

    pub fn reset(&mut self) {
        let mut idx = 0;
        while idx < MAX {
            self.domains[idx] = None;
            idx += 1;
        }
        self.population = 0;
    }

    #[inline(always)]
    fn hash(pid: ProcessId) -> usize {
        if MAX == 0 {
            return 0;
        }
        let raw = pid.raw();
        let mix = raw ^ (raw >> 33);
        (mix as usize) % MAX
    }

    #[inline(always)]
    fn next_index(current: usize) -> usize {
        if MAX == 0 {
            return 0;
        }
        (current + 1) % MAX
    }

    #[inline(always)]
    fn insert_domain(&mut self, domain: TaskDomain) -> Result<(), IsolationError> {
        if MAX == 0 {
            return Err(IsolationError::PolicyViolation);
        }

        let mut idx = Self::hash(domain.pid());
        let mut probes = 0;
        while probes < MAX {
            match self.domains[idx] {
                Some(existing) if existing.pid() == domain.pid() => {
                    self.domains[idx] = Some(domain);
                    return Ok(());
                }
                Some(_) => {
                    idx = Self::next_index(idx);
                }
                None => {
                    self.domains[idx] = Some(domain);
                    self.population += 1;
                    return Ok(());
                }
            }
            probes += 1;
        }

        Err(IsolationError::PolicyViolation)
    }

    #[inline(always)]
    fn lookup(&self, pid: ProcessId) -> Option<TaskDomain> {
        if MAX == 0 {
            return None;
        }

        let mut idx = Self::hash(pid);
        let mut probes = 0;
        while probes < MAX {
            match self.domains[idx] {
                Some(domain) if domain.pid() == pid => return Some(domain),
                Some(_) => {
                    idx = Self::next_index(idx);
                }
                None => return None,
            }
            probes += 1;
        }
        None
    }

    #[inline(always)]
    fn remove(&mut self, pid: ProcessId) {
        if MAX == 0 || self.population == 0 {
            return;
        }

        let mut idx = Self::hash(pid);
        let mut probes = 0;
        while probes < MAX {
            match self.domains[idx] {
                Some(domain) if domain.pid() == pid => {
                    self.domains[idx] = None;
                    self.population = self.population.saturating_sub(1);
                    self.rehash_from(Self::next_index(idx));
                    return;
                }
                Some(_) => {
                    idx = Self::next_index(idx);
                }
                None => return,
            }
            probes += 1;
        }
    }

    #[inline(always)]
    fn rehash_from(&mut self, mut idx: usize) {
        if MAX == 0 {
            return;
        }

        let mut probes = 0;
        while probes < MAX {
            match self.domains[idx] {
                None => return,
                Some(domain) => {
                    self.domains[idx] = None;
                    self.population = self.population.saturating_sub(1);
                    // reinsertion cannot fail because table has free slots after removal.
                    let _ = self.insert_domain(domain);
                    idx = Self::next_index(idx);
                }
            }
            probes += 1;
        }
    }

    pub fn register_task(
        &mut self,
        pid: ProcessId,
        creds: Credentials,
    ) -> Result<(), IsolationError> {
        self.insert_domain(TaskDomain::from_credentials(pid, creds))
    }

    #[inline(always)]
    pub fn revoke_task(&mut self, pid: ProcessId) {
        self.remove(pid);
    }

    #[inline(always)]
    pub fn authorize_ipc(
        &self,
        sender: ProcessId,
        receiver: ProcessId,
        class: SecurityClass,
    ) -> Result<(), IsolationError> {
        let sender_domain = self.domain(sender)?;
        let receiver_domain = self.domain(receiver)?;

        if !sender_domain.capabilities.allows_ipc() {
            return Err(IsolationError::CapabilityMissing);
        }

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

    #[inline(always)]
    pub fn authorize_device_access(
        &self,
        pid: ProcessId,
        security: DeviceSecurity,
    ) -> Result<(), IsolationError> {
        let domain = self.domain(pid)?;

        if !domain.capabilities.allows_io() {
            return Err(IsolationError::CapabilityMissing);
        }

        if security.requires_kernel_mode() && !domain.capabilities.allows_kernel_access() {
            return Err(IsolationError::CapabilityMissing);
        }

        if !domain.label.dominates(&security.class().as_label()) {
            return Err(IsolationError::PolicyViolation);
        }

        Ok(())
    }

    #[inline(always)]
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

    #[inline(always)]
    fn domain(&self, pid: ProcessId) -> Result<TaskDomain, IsolationError> {
        self.lookup(pid).ok_or(IsolationError::UnknownTask)
    }
}
