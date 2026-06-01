//! Low-level interrupt flag and halt helpers.

/// Disable maskable interrupts on the current CPU.
#[inline(always)]
pub fn disable() {
    #[cfg(not(test))]
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
    }
}

/// Enable maskable interrupts on the current CPU.
#[inline(always)]
pub fn enable() {
    #[cfg(not(test))]
    unsafe {
        core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
    }
}

/// Halt the current CPU until the next external interrupt arrives.
#[inline(always)]
pub fn halt() {
    #[cfg(not(test))]
    unsafe {
        core::arch::asm!("hlt", options(nomem, nostack));
    }

    #[cfg(test)]
    core::hint::spin_loop();
}

/// Disable interrupts and halt forever.
pub fn halt_forever() -> ! {
    disable();
    loop {
        halt();
    }
}
