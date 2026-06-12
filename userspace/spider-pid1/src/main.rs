#![no_std]
#![no_main]

const SYS_WRITE: usize = 2;
const SYS_YIELD: usize = 3;
const SYS_GETPID: usize = 4;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let pid = unsafe { syscall3(SYS_GETPID, 0, 0, 0) };
    if pid == 1 {
        write(1, b"Spider-rs PID 1 online\n");
    } else {
        write(2, b"Spider-rs PID check failed\n");
    }

    loop {
        let _ = unsafe { syscall3(SYS_YIELD, 0, 0, 0) };
        core::hint::spin_loop();
    }
}

fn write(fd: usize, bytes: &[u8]) {
    let _ = unsafe { syscall3(SYS_WRITE, fd, bytes.as_ptr() as usize, bytes.len()) };
}

#[inline(always)]
unsafe fn syscall3(number: usize, arg0: usize, arg1: usize, arg2: usize) -> isize {
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

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
