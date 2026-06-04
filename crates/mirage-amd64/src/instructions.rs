//! Feature-gated AMD64 hardware instructions.
//!
//! The functions in this module are the only place this crate uses unsafe code.
//! Callers must hold supervisor-granted authority for the underlying hardware
//! operation before invoking them.

use core::arch::asm;

use super::Msr;

/// Read a model-specific register.
///
/// # Safety
/// The caller must ensure that executing `rdmsr` for `msr` is valid at the
/// current CPU privilege level and that Mirage capability checks have already
/// authorized this hardware operation.
pub unsafe fn read_msr(msr: Msr) -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr.get(),
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    ((high as u64) << 32) | low as u64
}
