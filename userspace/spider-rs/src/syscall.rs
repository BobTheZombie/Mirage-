//! Mirage userspace syscall shim used by the no_std Spider-rs build.
//!
//! Keep these numbers synchronized with `src/kernel/syscall.rs`.  The older
//! experimental Spider ABI used small numbers 1-4; the kernel table now exposes
//! GetPid=0, Write=19, Exit=102.  Mirage does not have a dedicated userspace
//! yield syscall yet, so `yield_now` remains a local CPU hint until MTSS exposes
//! one.

pub const SYS_GETPID: usize = 0;
pub const SYS_WRITE: usize = 19;
pub const SYS_EXIT: usize = 102;

#[inline(always)]
pub unsafe fn syscall0(number: usize) -> isize {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: isize;
        core::arch::asm!(
            "syscall",
            inlateout("rax") number as isize => ret,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
        ret
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = number;
        -38
    }
}

#[inline(always)]
pub unsafe fn syscall1(number: usize, arg0: usize) -> isize {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: isize;
        core::arch::asm!(
            "syscall",
            inlateout("rax") number as isize => ret,
            in("rdi") arg0,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
        ret
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (number, arg0);
        -38
    }
}

#[inline(always)]
pub unsafe fn syscall3(number: usize, arg0: usize, arg1: usize, arg2: usize) -> isize {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: isize;
        core::arch::asm!(
            "syscall",
            inlateout("rax") number as isize => ret,
            in("rdi") arg0,
            in("rsi") arg1,
            in("rdx") arg2,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
        ret
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (number, arg0, arg1, arg2);
        -38
    }
}

pub fn write(fd: usize, bytes: &[u8]) -> isize {
    unsafe { syscall3(SYS_WRITE, fd, bytes.as_ptr() as usize, bytes.len()) }
}

pub fn yield_now() {
    // TODO(mtss): replace with a real MTSS yield syscall once the kernel ABI
    // assigns one.  Do not issue the old experimental syscall number 3 here;
    // that no longer maps to yield in the kernel table.
    core::hint::spin_loop();
}

pub fn getpid() -> isize {
    unsafe { syscall0(SYS_GETPID) }
}

pub fn exit(status: i32) -> ! {
    let _ = unsafe { syscall1(SYS_EXIT, status as usize) };
    loop {
        core::hint::spin_loop();
    }
}
