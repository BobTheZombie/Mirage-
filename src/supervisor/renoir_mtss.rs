//! Supervisor policy for Renoir MTSS scheduler module activation.
//!
//! The supervisor authorizes the selected scheduler module.  MTSS owns runnable
//! queues and execution mechanics; the supervisor must not directly mutate them.

use mirage_mtss::scheduler_modules::{
    select_scheduler_module, MtssCpuProfile, MtssSchedulerModuleDescriptor, MtssSchedulerModuleId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorSchedulerDecision {
    Approved(MtssSchedulerModuleDescriptor),
    Fallback(MtssSchedulerModuleDescriptor),
    Denied(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupervisorRenoirMtssPolicy {
    pub allow_arch_specific_modules: bool,
    pub allow_work_borrowing: bool,
    pub allow_cluster_execution: bool,
}

impl SupervisorRenoirMtssPolicy {
    pub const fn strict_default() -> Self {
        Self {
            allow_arch_specific_modules: true,
            allow_work_borrowing: true,
            allow_cluster_execution: false,
        }
    }

    pub const fn approve(self, cpu: MtssCpuProfile) -> SupervisorSchedulerDecision {
        let selected = select_scheduler_module(cpu);
        match selected.id {
            MtssSchedulerModuleId::GenericRoundRobin => {
                SupervisorSchedulerDecision::Fallback(selected)
            }
            MtssSchedulerModuleId::AmdZen2Renoir if !self.allow_arch_specific_modules => {
                SupervisorSchedulerDecision::Denied(
                    "arch-specific scheduler modules disabled by supervisor policy",
                )
            }
            MtssSchedulerModuleId::AmdZen2Renoir => SupervisorSchedulerDecision::Approved(selected),
        }
    }
}
