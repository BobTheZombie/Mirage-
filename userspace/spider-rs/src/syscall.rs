//! Mirage userspace syscall shim used by the no_std Spider-rs build.
//!
//! Keep these numbers synchronized with `src/kernel/syscall.rs`.  The older
//! experimental Spider ABI used small numbers 1-4; the kernel table now exposes
//! GetPid=0, Spawn=1, OpenAt=16, Close=17, Read=18, Write=19,
//! Getdents64=25, Exit=102, and Wait4=103. Mirage does not have a
//! dedicated userspace yield syscall yet, so `yield_now` remains a local CPU
//! hint until MTSS exposes one.

pub const SYS_GETPID: usize = 0;
pub const SYS_WRITE: usize = 19;
pub const SYS_EXIT: usize = 102;
pub const SYS_SPAWN: usize = 1;
pub const SYS_WAIT: usize = 103;
pub const SYS_OPENAT: usize = 16;
pub const SYS_CLOSE: usize = 17;
pub const SYS_READ: usize = 18;
pub const SYS_GETDENTS64: usize = 25;

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

#[inline(always)]
pub unsafe fn syscall4(number: usize, arg0: usize, arg1: usize, arg2: usize, arg3: usize) -> isize {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: isize;
        core::arch::asm!(
            "syscall",
            inlateout("rax") number as isize => ret,
            in("rdi") arg0,
            in("rsi") arg1,
            in("rdx") arg2,
            in("r10") arg3,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
        ret
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (number, arg0, arg1, arg2, arg3);
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

pub fn spawn(path: &str, argv: &[&str], _env: &[(&str, &str)]) -> Result<isize, isize> {
    let ret = unsafe {
        syscall4(
            SYS_SPAWN,
            path.as_ptr() as usize,
            path.len(),
            argv.as_ptr() as usize,
            argv.len(),
        )
    };
    if ret >= 0 {
        Ok(ret)
    } else {
        Err(ret)
    }
}

pub fn wait(pid: isize) -> Result<isize, isize> {
    let ret = unsafe { syscall1(SYS_WAIT, pid as usize) };
    if ret >= 0 {
        Ok(ret)
    } else {
        Err(ret)
    }
}

pub fn open(path: &str) -> Result<isize, isize> {
    // openat(AT_FDCWD, path, O_RDONLY, 0). The kernel path ABI consumes a
    // NUL-terminated userspace string, so build a bounded stack copy instead
    // of assuming Rust string literals carry a trailing NUL.
    const AT_FDCWD: usize = usize::MAX - 99;
    let bytes = path.as_bytes();
    if bytes.len() >= 256 {
        return Err(-36);
    }
    let mut nul_path = [0u8; 256];
    let mut index = 0usize;
    while index < bytes.len() {
        nul_path[index] = bytes[index];
        index += 1;
    }
    let ret = unsafe { syscall4(SYS_OPENAT, AT_FDCWD, nul_path.as_ptr() as usize, 0, 0) };
    if ret >= 0 {
        Ok(ret)
    } else {
        Err(ret)
    }
}

pub fn read(fd: usize, buffer: &mut [u8]) -> Result<usize, isize> {
    let ret = unsafe { syscall3(SYS_READ, fd, buffer.as_mut_ptr() as usize, buffer.len()) };
    if ret >= 0 {
        Ok(ret as usize)
    } else {
        Err(ret)
    }
}

pub fn close(fd: usize) -> Result<(), isize> {
    let ret = unsafe { syscall1(SYS_CLOSE, fd) };
    if ret >= 0 {
        Ok(())
    } else {
        Err(ret)
    }
}

pub fn read_dir(fd: usize, buffer: &mut [u8]) -> Result<usize, isize> {
    let ret = unsafe {
        syscall3(
            SYS_GETDENTS64,
            fd,
            buffer.as_mut_ptr() as usize,
            buffer.len(),
        )
    };
    if ret >= 0 {
        Ok(ret as usize)
    } else {
        Err(ret)
    }
}
