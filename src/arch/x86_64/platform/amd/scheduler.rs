//! Lower-kernel MTSS scheduler module selection for AMD Renoir.

use super::renoir::RenoirBootProfile;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenoirSchedulerSelection {
    pub module_name: &'static str,
    pub reason: &'static str,
}

pub const fn select_renoir_scheduler_module(
    profile: Option<RenoirBootProfile>,
) -> RenoirSchedulerSelection {
    match profile {
        Some(profile) if profile.cpuid.is_renoir() => RenoirSchedulerSelection {
            module_name: "mtss-sched-amd-zen2-renoir",
            reason: "Renoir/Zen 2 mobile profile detected by lower kernel CPUID probe",
        },
        _ => RenoirSchedulerSelection {
            module_name: "mtss-sched-generic-round-robin",
            reason: "no Renoir platform profile available",
        },
    }
}
