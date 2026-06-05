#![no_std]
#![cfg_attr(not(feature = "hw-amd64"), forbid(unsafe_code))]
#![deny(unsafe_op_in_unsafe_fn)]

//! AMD64 mechanism primitives for Mirage.
//!
//! This crate deliberately contains low-level architectural mechanism only:
//! descriptor types, control-register bit definitions, capability-guarded MSR
//! access, and feature-gated raw instruction access. Platform policy, service launch choices, and recovery
//! decisions belong in the supervisor/platform layer.

/// Architectural CPU privilege ring.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum PrivilegeRing {
    Ring0,
    Ring1,
    Ring2,
    Ring3,
}

/// A raw model-specific register number used by the instruction backend.
///
/// Supervisor/platform MSR access should prefer [`msr::MsrAddress`], which
/// rejects unknown addresses and enforces Mirage capability checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Msr(u32);

impl Msr {
    pub const EFER: Self = Self(0xc000_0080);
    pub const STAR: Self = Self(0xc000_0081);
    pub const LSTAR: Self = Self(0xc000_0082);
    pub const FMASK: Self = Self(0xc000_0084);

    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Control-register bit masks surfaced as pure data for kernel setup code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlRegisterBits;

impl ControlRegisterBits {
    pub const CR0_PROTECTED_MODE: u64 = 1 << 0;
    pub const CR0_PAGING: u64 = 1 << 31;
    pub const CR4_PAE: u64 = 1 << 5;
    pub const CR4_OSFXSR: u64 = 1 << 9;
    pub const CR4_OSXMMEXCPT: u64 = 1 << 10;
    pub const EFER_LONG_MODE_ENABLE: u64 = 1 << 8;
    pub const EFER_LONG_MODE_ACTIVE: u64 = 1 << 10;
    pub const EFER_NO_EXECUTE_ENABLE: u64 = 1 << 11;
}

/// A checked physical address aligned for AMD64 page-table roots.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PageTableRoot(u64);

impl PageTableRoot {
    pub const ALIGNMENT: u64 = 4096;

    pub const fn new(physical_address: u64) -> Result<Self, Amd64Error> {
        if physical_address & (Self::ALIGNMENT - 1) != 0 {
            Err(Amd64Error::UnalignedPageTableRoot)
        } else {
            Ok(Self(physical_address))
        }
    }

    pub const fn physical_address(self) -> u64 {
        self.0
    }
}

/// Minimal syscall entry descriptor consumed by kernel mechanism code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyscallEntry {
    pub kernel_code_selector: u16,
    pub user_code_selector: u16,
    pub entry_point: u64,
    pub flags_mask: u64,
}

impl SyscallEntry {
    pub const fn new(
        kernel_code_selector: u16,
        user_code_selector: u16,
        entry_point: u64,
        flags_mask: u64,
    ) -> Self {
        Self {
            kernel_code_selector,
            user_code_selector,
            entry_point,
            flags_mask,
        }
    }
}

/// Errors produced by pure AMD64 mechanism validators.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Amd64Error {
    UnalignedPageTableRoot,
}

pub mod msr;

#[cfg(all(feature = "hw-amd64", target_arch = "x86_64"))]
pub mod instructions;

#[cfg(any(not(feature = "hw-amd64"), not(target_arch = "x86_64")))]
pub mod instructions {
    //! Stub instruction backend used for mock builds and non-AMD64 hosts.

    use super::Msr;

    /// Raw hardware instruction access is unavailable in this build.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum InstructionError {
        Unavailable,
    }

    pub fn read_msr(_msr: Msr) -> Result<u64, InstructionError> {
        Err(InstructionError::Unavailable)
    }
}

pub mod cache;
pub mod cpuid;
pub mod topology;

pub use cache::AmdCacheInfo;
pub use cpuid::{
    AmdCpuFamily, AmdCpuId, AmdCpuModel, AmdCpuStepping, AmdCpuidReader, AmdFeatureSet, AmdVendor,
    CpuidLeaf, HardwareCpuid,
};
pub use topology::{AmdCoreId, AmdPackageId, AmdThreadId, AmdTopology};
