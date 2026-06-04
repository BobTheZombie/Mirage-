//! Feature-gated AMD64 hardware instructions.
//!
//! Callers must hold supervisor-granted authority for the underlying hardware
//! operation before invoking these raw instruction wrappers. Prefer the safe,
//! capability-guarded `msr` module for supervisor/platform MSR access.

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
    // SAFETY: The caller has promised that `rdmsr` is valid and authorized
    // for this MSR at the current CPU privilege level; this block contains
    // only the raw instruction needed to perform that read.
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

/// Write a model-specific register.
///
/// # Safety
/// The caller must ensure that executing `wrmsr` for `msr` is valid at the
/// current CPU privilege level, that the value is architecturally valid for the
/// target MSR, and that Mirage capability checks have already authorized this
/// hardware operation.
pub unsafe fn write_msr(msr: Msr, value: u64) {
    // SAFETY: The caller has promised that `wrmsr` is valid and authorized
    // for this MSR/value at the current CPU privilege level; this block
    // contains only the raw instruction needed to perform that write.
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr.get(),
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nomem, nostack, preserves_flags),
        );
    }
}
