//! x86_64 PCI configuration-space access.
//!
//! This module contains the legacy PCI mechanism #1 I/O-port path using
//! `0xCF8` (CONFIG_ADDRESS) and `0xCFC` (CONFIG_DATA). Generic PCI parsing and
//! enumeration lives outside this module so raw port authority can remain behind
//! Mirage supervisor capabilities.

use core::arch::asm;

use crate::{PciAddress, PciConfigAccess, PciError};

const CONFIG_ADDRESS_PORT: u16 = 0x0cf8;
const CONFIG_DATA_PORT: u16 = 0x0cfc;
const ENABLE_BIT: u32 = 1 << 31;

/// Legacy x86_64 PCI config-space backend using I/O ports `0xCF8` / `0xCFC`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegacyPciConfigPorts {
    raw_port_authority: (),
}

impl LegacyPciConfigPorts {
    /// Creates a legacy port-I/O PCI backend after the caller has obtained raw
    /// config-port authority from the Mirage supervisor.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that this execution context has permission to
    /// access I/O ports `0xCF8` and `0xCFC`, and that using legacy PCI mechanism
    /// #1 is correct for the current machine.
    pub const unsafe fn new_unchecked() -> Self {
        Self {
            raw_port_authority: (),
        }
    }

    pub const fn config_address(address: PciAddress, offset: u16) -> Result<u32, PciError> {
        if offset > 0xfc || offset % 4 != 0 {
            return Err(PciError::InvalidConfigOffset);
        }

        Ok(ENABLE_BIT
            | ((address.bus() as u32) << 16)
            | ((address.device() as u32) << 11)
            | ((address.function() as u32) << 8)
            | ((offset as u32) & 0xfc))
    }

    /// Writes one aligned PCI config dword through the legacy x86_64 I/O ports.
    ///
    /// # Safety
    ///
    /// The caller must ensure the write is valid for the target device and does
    /// not bypass supervisor policy for device enablement or BAR programming.
    pub unsafe fn write_u32_unchecked(
        &self,
        address: PciAddress,
        offset: u16,
        value: u32,
    ) -> Result<(), PciError> {
        let config_address = Self::config_address(address, offset)?;
        unsafe {
            outl(CONFIG_ADDRESS_PORT, config_address);
            outl(CONFIG_DATA_PORT, value);
        }
        Ok(())
    }
}

impl PciConfigAccess for LegacyPciConfigPorts {
    fn read_u32(&self, address: PciAddress, offset: u16) -> Result<u32, PciError> {
        let config_address = Self::config_address(address, offset)?;
        unsafe {
            outl(CONFIG_ADDRESS_PORT, config_address);
            Ok(inl(CONFIG_DATA_PORT))
        }
    }
}

unsafe fn outl(port: u16, value: u32) {
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
    }
}

unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    unsafe {
        asm!("in eax, dx", out("eax") value, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    value
}
