#![no_std]
#![forbid(unsafe_code)]

//! Ryzen-specific mechanism descriptors.
//!
//! This crate models CPU/package facts that the supervisor can use to make
//! policy decisions elsewhere. It does not choose drivers or grant authority.

use mirage_amd64::PrivilegeRing;

/// AMD CPU family/model/stepping tuple decoded by architecture probing code.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RyzenCpuId {
    family: u16,
    model: u16,
    stepping: u8,
}

impl RyzenCpuId {
    pub const fn new(family: u16, model: u16, stepping: u8) -> Self {
        Self {
            family,
            model,
            stepping,
        }
    }

    pub const fn family(self) -> u16 {
        self.family
    }

    pub const fn model(self) -> u16 {
        self.model
    }

    pub const fn stepping(self) -> u8 {
        self.stepping
    }
}

/// Topology facts surfaced by low-level discovery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RyzenTopology {
    pub packages: u16,
    pub cores_per_package: u16,
    pub threads_per_core: u16,
}

impl RyzenTopology {
    pub const fn new(packages: u16, cores_per_package: u16, threads_per_core: u16) -> Self {
        Self {
            packages,
            cores_per_package,
            threads_per_core,
        }
    }

    pub const fn logical_cpus(self) -> u32 {
        self.packages as u32 * self.cores_per_package as u32 * self.threads_per_core as u32
    }
}

/// Telemetry channel identifiers exposed as mechanism, not policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RyzenTelemetryChannel {
    TemperatureCelsius,
    PackagePowerMilliwatts,
    CoreVoltageMillivolts,
}

/// A low-level Ryzen hardware profile discovered before supervisor policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RyzenProfile {
    pub cpu_id: RyzenCpuId,
    pub topology: RyzenTopology,
    pub required_ring: PrivilegeRing,
}

impl RyzenProfile {
    pub const fn new(cpu_id: RyzenCpuId, topology: RyzenTopology) -> Self {
        Self {
            cpu_id,
            topology,
            required_ring: PrivilegeRing::Ring0,
        }
    }
}
