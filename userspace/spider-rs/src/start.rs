//! `_start` entry for the no_std Spider-rs userspace ELF.

use crate::syscall;

pub fn spider_main() -> ! {
    let _ = syscall::write(1, b"Mirage M1.1 System\n");
    let _ = syscall::write(1, b"hello world\n");
    syscall::exit(0);
}
