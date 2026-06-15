#![allow(dead_code)]

//! AMD Renoir / Ryzen 5 4500U platform descriptors.
//!
//! This module is deliberately mechanism-only.  It gives the kernel,
//! MTSS, and supervisor a stable description of a Renoir-class Zen 2
//! mobile platform without claiming that any device driver is online.

use crate::{RyzenCpuId, RyzenGeneration, RyzenSocKind, RyzenTopology};

pub const RYZEN_4500U_MARKETING_NAME: &str = "AMD Ryzen 5 4500U";
pub const RENOIR_FAMILY: u16 = 0x17;
pub const RENOIR_MODEL_MIN: u16 = 0x60;
pub const RENOIR_MODEL_MAX: u16 = 0x7f;
pub const RYZEN_4500U_CORES: u16 = 6;
pub const RYZEN_4500U_THREADS: u16 = 6;
pub const RYZEN_4500U_BASE_MHZ: u32 = 2300;
pub const RYZEN_4500U_BOOST_MHZ: u32 = 4000;
pub const RYZEN_4500U_TDP_WATTS: u16 = 15;
pub const RYZEN_4500U_SOCKET: &str = "FP6";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenoirDetectionKind {
    ExactRyzen4500UClass,
    RenoirZen2Mobile,
    UnsupportedAmd64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenoirCpuProfile {
    pub cpu_id: RyzenCpuId,
    pub topology: RyzenTopology,
    pub generation: RyzenGeneration,
    pub soc_kind: RyzenSocKind,
    pub detection: RenoirDetectionKind,
}

impl RenoirCpuProfile {
    pub const fn from_parts(cpu_id: RyzenCpuId, topology: RyzenTopology) -> Self {
        let is_renoir = cpu_id.family() == RENOIR_FAMILY
            && cpu_id.model() >= RENOIR_MODEL_MIN
            && cpu_id.model() <= RENOIR_MODEL_MAX;
        let exact_4500u = is_renoir
            && topology.cores_per_package == RYZEN_4500U_CORES
            && topology.threads_per_core == 1;
        let detection = if exact_4500u {
            RenoirDetectionKind::ExactRyzen4500UClass
        } else if is_renoir {
            RenoirDetectionKind::RenoirZen2Mobile
        } else {
            RenoirDetectionKind::UnsupportedAmd64
        };
        Self {
            cpu_id,
            topology,
            generation: if is_renoir {
                RyzenGeneration::Zen2
            } else {
                RyzenGeneration::UnknownAmd64
            },
            soc_kind: if is_renoir {
                RyzenSocKind::Mobile
            } else {
                RyzenSocKind::Unknown
            },
            detection,
        }
    }

    pub const fn is_renoir(self) -> bool {
        matches!(
            self.detection,
            RenoirDetectionKind::ExactRyzen4500UClass | RenoirDetectionKind::RenoirZen2Mobile
        )
    }

    pub const fn is_ryzen_4500u_class(self) -> bool {
        matches!(self.detection, RenoirDetectionKind::ExactRyzen4500UClass)
    }

    pub const fn scheduler_module_name(self) -> &'static str {
        if self.is_renoir() {
            "mtss-sched-amd-zen2-renoir"
        } else {
            "mtss-sched-generic"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_ryzen_4500u_class() {
        let profile = RenoirCpuProfile::from_parts(
            RyzenCpuId::new(0x17, 0x60, 1),
            RyzenTopology::new(1, 6, 1),
        );
        assert!(profile.is_ryzen_4500u_class());
        assert_eq!(profile.scheduler_module_name(), "mtss-sched-amd-zen2-renoir");
    }
}
