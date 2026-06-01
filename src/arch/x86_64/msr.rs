//! Model-specific register helpers used during CPU bootstrap.

pub const IA32_EFER: u32 = 0xc000_0080;
pub const IA32_STAR: u32 = 0xc000_0081;
pub const IA32_LSTAR: u32 = 0xc000_0082;
pub const IA32_FMASK: u32 = 0xc000_0084;

const EFER_SYSCALL_ENABLE: u64 = 1;
const RFLAGS_INTERRUPT_ENABLE: u64 = 1 << 9;
const RFLAGS_DIRECTION: u64 = 1 << 10;

#[inline(always)]
pub unsafe fn read(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") low,
        out("edx") high,
        options(nomem, nostack, preserves_flags),
    );
    ((high as u64) << 32) | low as u64
}

#[inline(always)]
pub unsafe fn write(msr: u32, value: u64) {
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") value as u32,
        in("edx") (value >> 32) as u32,
        options(nomem, nostack, preserves_flags),
    );
}

/// Enable the `syscall` instruction and point it at the supplied kernel entry stub.
pub fn enable_syscall_entry(entry: usize, kernel_code_selector: u16, user_code_selector: u16) {
    #[cfg(not(test))]
    unsafe {
        let efer = read(IA32_EFER);
        write(IA32_EFER, efer | EFER_SYSCALL_ENABLE);

        let star = ((user_code_selector as u64 - 16) << 48) | ((kernel_code_selector as u64) << 32);
        write(IA32_STAR, star);
        write(IA32_LSTAR, entry as u64);
        write(IA32_FMASK, RFLAGS_INTERRUPT_ENABLE | RFLAGS_DIRECTION);
    }

    #[cfg(test)]
    let _ = (entry, kernel_code_selector, user_code_selector);
}
