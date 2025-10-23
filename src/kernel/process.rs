//! Process control structures for the Mirage kernel.

use crate::subkernel::SecurityLabel;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProcessId(u64);

impl ProcessId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    pub const fn raw(&self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    Blocked,
    Terminated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessPriority {
    Critical,
    High,
    Normal,
    Low,
}

impl ProcessPriority {
    pub const fn time_slice(self) -> u8 {
        match self {
            ProcessPriority::Critical => 8,
            ProcessPriority::High => 6,
            ProcessPriority::Normal => 4,
            ProcessPriority::Low => 2,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessControlBlock {
    pub pid: ProcessId,
    pub parent: Option<ProcessId>,
    pub state: ProcessState,
    pub priority: ProcessPriority,
    pub entry_point: u64,
    pub address_space_root: u64,
    pub cpu_time: u128,
    pub security_label: SecurityLabel,
}

impl ProcessControlBlock {
    pub const fn new(
        pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
        parent: Option<ProcessId>,
    ) -> Self {
        Self {
            pid,
            parent,
            state: ProcessState::Ready,
            priority,
            entry_point,
            address_space_root: 0,
            cpu_time: 0,
            security_label: SecurityLabel::public(),
        }
    }

    pub fn update_security_label(&mut self, label: SecurityLabel) {
        self.security_label = label;
    }
}

impl core::fmt::Display for ProcessId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PartialEq<u64> for ProcessId {
    fn eq(&self, other: &u64) -> bool {
        self.0 == *other
    }
}
