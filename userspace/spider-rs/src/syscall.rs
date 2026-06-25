//! Mirage userspace syscall shim used by the no_std Spider-rs build.
//!
//! Syscall numbers are imported from the shared `mirage-abi` crate so the
//! no_std Spider runtime and kernel consume the same append-only ABI table.

pub use mirage_abi::syscall::{
    SYS_CLOSE, SYS_EXIT, SYS_GETDENTS64, SYS_GETPID, SYS_OPENAT, SYS_READ, SYS_SPAWN, SYS_WAIT,
    SYS_WRITE, SYS_YIELD,
};

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
    let _ = unsafe { syscall0(SYS_YIELD) };
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

#[cfg(test)]
mod tests {
    use super::*;
    use mirage_abi::syscall as abi;

    #[test]
    fn userspace_syscall_constants_match_shared_kernel_abi() {
        assert_eq!(SYS_GETPID as u64, abi::MIRAGE_SYSCALL_GETPID);
        assert_eq!(SYS_SPAWN as u64, abi::MIRAGE_SYSCALL_SPAWN);
        assert_eq!(SYS_OPENAT as u64, abi::MIRAGE_SYSCALL_OPENAT);
        assert_eq!(SYS_CLOSE as u64, abi::MIRAGE_SYSCALL_CLOSE);
        assert_eq!(SYS_READ as u64, abi::MIRAGE_SYSCALL_READ);
        assert_eq!(SYS_WRITE as u64, abi::MIRAGE_SYSCALL_WRITE);
        assert_eq!(SYS_GETDENTS64 as u64, abi::MIRAGE_SYSCALL_GETDENTS64);
        assert_eq!(SYS_EXIT as u64, abi::MIRAGE_SYSCALL_EXIT);
        assert_eq!(SYS_WAIT as u64, abi::MIRAGE_SYSCALL_WAIT4);
        assert_eq!(SYS_YIELD as u64, abi::MIRAGE_SYSCALL_YIELD);
    }
}
