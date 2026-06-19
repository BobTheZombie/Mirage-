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

/// Return whether maskable interrupts are enabled on the current CPU.
#[inline(always)]
pub fn are_enabled() -> bool {
    #[cfg(not(test))]
    unsafe {
        let flags: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags));
        flags & (1 << 9) != 0
    }

    #[cfg(test)]
    {
        false
    }
}

/// Run a short critical section with maskable interrupts disabled, restoring
/// the previous interrupt-enable state afterwards.
#[inline(always)]
pub fn without_interrupts<T>(f: impl FnOnce() -> T) -> T {
    let was_enabled = are_enabled();
    disable();
    let result = f();
    if was_enabled {
        enable();
    }
    result
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
