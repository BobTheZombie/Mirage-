//! Capability-guarded AMD64 model-specific register access.
//!
//! This module is the supervisor/platform-facing MSR abstraction. It rejects
//! arbitrary numeric MSR addresses and only exposes named MSRs that Mirage has
//! deliberately modeled. Write access is additionally guarded by an explicit
//! [`MsrCapability`] for the same named register.
//!
//! The early bootstrap helper in `src/arch/x86_64/msr.rs` remains a
//! kernel-private mechanism for CPU bring-up before the supervisor/platform
//! authority model is online. Once Mirage is past that bootstrap phase,
//! supervisor and platform code should route MSR operations through this
//! capability-checked abstraction instead of passing raw addresses directly to
//! `rdmsr`/`wrmsr`.

use core::convert::TryFrom;

/// A named AMD64 model-specific register address accepted by Mirage.
///
/// The raw address is intentionally private: callers must use one of the named
/// constants or [`TryFrom<u32>`], which rejects unknown MSRs instead of allowing
/// arbitrary hardware access.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MsrAddress(u32);

impl MsrAddress {
    /// Extended Feature Enable Register (`IA32_EFER`).
    pub const EFER: Self = Self(0xc000_0080);
    /// Local APIC base register (`IA32_APIC_BASE`).
    pub const APIC_BASE: Self = Self(0x0000_001b);
    /// Time Stamp Counter (`IA32_TSC`).
    pub const TSC: Self = Self(0x0000_0010);
    /// TSC deadline register (`IA32_TSC_DEADLINE`).
    pub const TSC_DEADLINE: Self = Self(0x0000_06e0);
    /// AMD Hardware Configuration Register (`HWCR`). Read-only in Mirage.
    pub const HWCR: Self = Self(0xc001_0015);

    /// Return the architectural numeric MSR address.
    pub const fn raw(self) -> u32 {
        self.0
    }

    const fn is_read_only(self) -> bool {
        matches!(self, Self::HWCR)
    }
}

impl TryFrom<u32> for MsrAddress {
    type Error = MsrAccessError;

    fn try_from(raw: u32) -> Result<Self, Self::Error> {
        match raw {
            0xc000_0080 => Ok(Self::EFER),
            0x0000_001b => Ok(Self::APIC_BASE),
            0x0000_0010 => Ok(Self::TSC),
            0x0000_06e0 => Ok(Self::TSC_DEADLINE),
            0xc001_0015 => Ok(Self::HWCR),
            other => Err(MsrAccessError::UnknownMsr(other)),
        }
    }
}

/// A 64-bit model-specific register value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MsrValue(u64);

impl MsrValue {
    /// Construct an MSR value from raw bits.
    pub const fn new(bits: u64) -> Self {
        Self(bits)
    }

    /// Return the raw 64-bit MSR value.
    pub const fn bits(self) -> u64 {
        self.0
    }
}

/// Access class requested for an MSR operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MsrOperation {
    Read,
    Write,
}

/// Errors returned by capability-guarded MSR operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MsrAccessError {
    /// The requested raw MSR address is not in Mirage's named allow-list.
    UnknownMsr(u32),
    /// The supplied capability does not grant the requested operation on the MSR.
    PermissionDenied {
        address: MsrAddress,
        operation: MsrOperation,
    },
    /// Mirage models this MSR as read-only and will not issue `wrmsr` for it.
    ReadOnly(MsrAddress),
    /// Hardware MSR instructions are unavailable in this build/target.
    HardwareUnavailable,
}

/// Explicit authority for one named MSR.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MsrCapability {
    address: MsrAddress,
    read: bool,
    write: bool,
}

impl MsrCapability {
    /// Grant read-only access to a specific named MSR.
    pub const fn read_only(address: MsrAddress) -> Self {
        Self {
            address,
            read: true,
            write: false,
        }
    }

    /// Grant read/write access to a specific named MSR.
    ///
    /// Write attempts still fail for MSRs that Mirage models as read-only.
    pub const fn read_write(address: MsrAddress) -> Self {
        Self {
            address,
            read: true,
            write: true,
        }
    }

    /// The named MSR this capability covers.
    pub const fn address(self) -> MsrAddress {
        self.address
    }

    const fn grants_read(self, address: MsrAddress) -> bool {
        self.read && self.address.0 == address.0
    }

    const fn grants_write(self, address: MsrAddress) -> bool {
        self.write && self.address.0 == address.0
    }
}

/// Read a named MSR after verifying the supplied capability.
pub fn read_msr(
    address: MsrAddress,
    capability: &MsrCapability,
) -> Result<MsrValue, MsrAccessError> {
    if !capability.grants_read(address) {
        return Err(MsrAccessError::PermissionDenied {
            address,
            operation: MsrOperation::Read,
        });
    }

    backend::read(address)
}

/// Write a named MSR after verifying the supplied capability.
pub fn write_msr(
    address: MsrAddress,
    value: MsrValue,
    capability: &MsrCapability,
) -> Result<(), MsrAccessError> {
    if address.is_read_only() {
        return Err(MsrAccessError::ReadOnly(address));
    }

    if !capability.grants_write(address) {
        return Err(MsrAccessError::PermissionDenied {
            address,
            operation: MsrOperation::Write,
        });
    }

    backend::write(address, value)
}

/// Validate a raw MSR address, then read it with capability checks.
pub fn try_read_msr(
    raw_address: u32,
    capability: &MsrCapability,
) -> Result<MsrValue, MsrAccessError> {
    read_msr(MsrAddress::try_from(raw_address)?, capability)
}

/// Validate a raw MSR address, then write it with capability checks.
pub fn try_write_msr(
    raw_address: u32,
    value: MsrValue,
    capability: &MsrCapability,
) -> Result<(), MsrAccessError> {
    write_msr(MsrAddress::try_from(raw_address)?, value, capability)
}

#[cfg(all(feature = "hw-amd64", target_arch = "x86_64", not(test)))]
mod backend {
    use super::{MsrAccessError, MsrAddress, MsrValue};
    use crate::{instructions, Msr};

    pub(super) fn read(address: MsrAddress) -> Result<MsrValue, MsrAccessError> {
        // SAFETY: `address` can only be constructed from Mirage's named MSR
        // allow-list, and the public caller path has already checked that the
        // supplied `MsrCapability` grants read authority for this exact MSR.
        let value = unsafe { instructions::read_msr(Msr::new(address.raw())) };
        Ok(MsrValue::new(value))
    }

    pub(super) fn write(address: MsrAddress, value: MsrValue) -> Result<(), MsrAccessError> {
        // SAFETY: `address` can only be constructed from Mirage's named MSR
        // allow-list, and the public caller path has already checked both that
        // the MSR is not modeled as read-only and that the supplied
        // `MsrCapability` grants write authority for this exact MSR.
        unsafe { instructions::write_msr(Msr::new(address.raw()), value.bits()) };
        Ok(())
    }
}

#[cfg(any(not(feature = "hw-amd64"), not(target_arch = "x86_64"), test))]
mod backend {
    use core::sync::atomic::{AtomicU64, Ordering};

    use super::{MsrAccessError, MsrAddress, MsrValue};

    static EFER: AtomicU64 = AtomicU64::new(0);
    static APIC_BASE: AtomicU64 = AtomicU64::new(0);
    static TSC: AtomicU64 = AtomicU64::new(0);
    static TSC_DEADLINE: AtomicU64 = AtomicU64::new(0);
    static HWCR: AtomicU64 = AtomicU64::new(0x0000_0000_0000_0001);

    pub(super) fn read(address: MsrAddress) -> Result<MsrValue, MsrAccessError> {
        Ok(MsrValue::new(cell(address).load(Ordering::SeqCst)))
    }

    pub(super) fn write(address: MsrAddress, value: MsrValue) -> Result<(), MsrAccessError> {
        cell(address).store(value.bits(), Ordering::SeqCst);
        Ok(())
    }

    fn cell(address: MsrAddress) -> &'static AtomicU64 {
        match address.raw() {
            0xc000_0080 => &EFER,
            0x0000_001b => &APIC_BASE,
            0x0000_0010 => &TSC,
            0x0000_06e0 => &TSC_DEADLINE,
            0xc001_0015 => &HWCR,
            _ => unreachable!("MsrAddress only exposes Mirage-named MSRs"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_backend_reads_and_writes_named_msrs() {
        let capability = MsrCapability::read_write(MsrAddress::EFER);
        let value = MsrValue::new(0xdead_beef_cafe_babe);

        write_msr(MsrAddress::EFER, value, &capability).expect("EFER write should be allowed");
        assert_eq!(read_msr(MsrAddress::EFER, &capability), Ok(value));
    }

    #[test]
    fn write_requires_explicit_capability_for_same_msr() {
        let read_only = MsrCapability::read_only(MsrAddress::EFER);
        assert_eq!(
            write_msr(MsrAddress::EFER, MsrValue::new(1), &read_only),
            Err(MsrAccessError::PermissionDenied {
                address: MsrAddress::EFER,
                operation: MsrOperation::Write,
            })
        );

        let wrong_msr = MsrCapability::read_write(MsrAddress::APIC_BASE);
        assert_eq!(
            write_msr(MsrAddress::EFER, MsrValue::new(1), &wrong_msr),
            Err(MsrAccessError::PermissionDenied {
                address: MsrAddress::EFER,
                operation: MsrOperation::Write,
            })
        );
    }

    #[test]
    fn raw_unknown_msrs_are_rejected() {
        let capability = MsrCapability::read_write(MsrAddress::EFER);

        assert_eq!(
            try_read_msr(0xffff_ffff, &capability),
            Err(MsrAccessError::UnknownMsr(0xffff_ffff))
        );
        assert_eq!(
            try_write_msr(0xffff_ffff, MsrValue::new(0), &capability),
            Err(MsrAccessError::UnknownMsr(0xffff_ffff))
        );
    }

    #[test]
    fn read_only_msrs_reject_writes_even_with_write_capability() {
        let capability = MsrCapability::read_write(MsrAddress::HWCR);

        assert_eq!(
            write_msr(MsrAddress::HWCR, MsrValue::new(0), &capability),
            Err(MsrAccessError::ReadOnly(MsrAddress::HWCR))
        );
    }
}
