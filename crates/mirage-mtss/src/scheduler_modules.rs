#![allow(dead_code)]

//! MTSS scheduler module registry.
//!
//! MTSS remains the execution fabric.  Scheduler modules are policy plugins
//! selected from lower-kernel platform facts and authorized by supervisor policy.
//! They do not directly mutate remote per-core queues.

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum MtssSchedulerModuleId {
    GenericRoundRobin,
    AmdZen2Renoir,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MtssSchedulerModuleState {
    Registered,
    Selected,
    Online,
    Stub,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MtssCpuProfile {
    pub family: u16,
    pub model: u16,
    pub cores: u16,
    pub threads: u16,
    pub amd: bool,
    pub renoir: bool,
}

impl MtssCpuProfile {
    pub const fn generic() -> Self {
        Self {
            family: 0,
            model: 0,
            cores: 1,
            threads: 1,
            amd: false,
            renoir: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MtssSchedulerModuleDescriptor {
    pub id: MtssSchedulerModuleId,
    pub name: &'static str,
    pub arch: &'static str,
    pub microarch: &'static str,
    pub state: MtssSchedulerModuleState,
    pub supports_core_local_queues: bool,
    pub supports_work_borrowing: bool,
    pub supports_cluster_execution: bool,
    pub supports_helper_packs: bool,
}

pub trait MtssSchedulerModule {
    fn descriptor(&self) -> MtssSchedulerModuleDescriptor;
    fn supports(&self, cpu: MtssCpuProfile) -> bool;
    fn pick_next_policy_name(&self) -> &'static str;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenericRoundRobinScheduler;

impl MtssSchedulerModule for GenericRoundRobinScheduler {
    fn descriptor(&self) -> MtssSchedulerModuleDescriptor {
        GENERIC_ROUND_ROBIN_DESCRIPTOR
    }

    fn supports(&self, _cpu: MtssCpuProfile) -> bool {
        true
    }

    fn pick_next_policy_name(&self) -> &'static str {
        "round-robin"
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AmdZen2RenoirScheduler;

impl MtssSchedulerModule for AmdZen2RenoirScheduler {
    fn descriptor(&self) -> MtssSchedulerModuleDescriptor {
        AMD_ZEN2_RENOIR_DESCRIPTOR
    }

    fn supports(&self, cpu: MtssCpuProfile) -> bool {
        cpu.amd && cpu.renoir && cpu.family == 0x17 && cpu.model >= 0x60 && cpu.model <= 0x7f
    }

    fn pick_next_policy_name(&self) -> &'static str {
        "amd-zen2-renoir-cache-local"
    }
}

pub const GENERIC_ROUND_ROBIN_DESCRIPTOR: MtssSchedulerModuleDescriptor =
    MtssSchedulerModuleDescriptor {
        id: MtssSchedulerModuleId::GenericRoundRobin,
        name: "mtss-sched-generic-round-robin",
        arch: "generic",
        microarch: "generic",
        state: MtssSchedulerModuleState::Online,
        supports_core_local_queues: true,
        supports_work_borrowing: false,
        supports_cluster_execution: false,
        supports_helper_packs: false,
    };

pub const AMD_ZEN2_RENOIR_DESCRIPTOR: MtssSchedulerModuleDescriptor =
    MtssSchedulerModuleDescriptor {
        id: MtssSchedulerModuleId::AmdZen2Renoir,
        name: "mtss-sched-amd-zen2-renoir",
        arch: "x86_64",
        microarch: "amd-zen2-renoir",
        state: MtssSchedulerModuleState::Registered,
        supports_core_local_queues: true,
        supports_work_borrowing: true,
        supports_cluster_execution: true,
        supports_helper_packs: true,
    };

pub const SCHEDULER_MODULES: &[MtssSchedulerModuleDescriptor] =
    &[GENERIC_ROUND_ROBIN_DESCRIPTOR, AMD_ZEN2_RENOIR_DESCRIPTOR];

pub const fn select_scheduler_module(cpu: MtssCpuProfile) -> MtssSchedulerModuleDescriptor {
    if cpu.amd && cpu.renoir && cpu.family == 0x17 && cpu.model >= 0x60 && cpu.model <= 0x7f {
        AMD_ZEN2_RENOIR_DESCRIPTOR
    } else {
        GENERIC_ROUND_ROBIN_DESCRIPTOR
    }
}
