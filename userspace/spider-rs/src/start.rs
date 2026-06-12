//! `_start` entry for a future no_std Spider-rs userspace ELF.

use crate::syscall;

pub fn spider_main() -> ! {
    let _ = syscall::write(1, b"Spider-rs PID 1 online\n");
    let _ = syscall::write(1, b"Spider-rs: loading units\n");
    let _ = syscall::write(1, b"Spider-rs: default.target reached\n");
    let _ = syscall::write(1, b"Spider-rs: no real services started yet\n");
    loop {
        syscall::yield_now();
    }
}

#[cfg(target_os = "none")]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    spider_main()
}
