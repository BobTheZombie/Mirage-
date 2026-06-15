//! Mirage userspace syscall shim used by the eventual no_std Spider-rs build.

pub const SYS_EXIT: usize = 1;
pub const SYS_WRITE: usize = 2;
pub const SYS_YIELD: usize = 3;
pub const SYS_GETPID: usize = 4;

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
    let _ = unsafe { syscall3(SYS_YIELD, 0, 0, 0) };
}

pub fn getpid() -> isize {
    unsafe { syscall3(SYS_GETPID, 0, 0, 0) }
}

pub fn exit(status: i32) -> ! {
    let _ = unsafe { syscall3(SYS_EXIT, status as usize, 0, 0) };
    loop {
        core::hint::spin_loop();
    }
}
