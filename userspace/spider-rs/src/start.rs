//! `_start` entry for the no_std Spider-rs userspace ELF.

use crate::{log, service, syscall};

pub fn spider_main() -> ! {
    log::info("Spider-rs PID 1 online");
    service::activate_builtin_graph();
    log::info("Spider-rs: entering service manager loop");
    loop {
        syscall::yield_now();
        core::hint::spin_loop();
    }
}
