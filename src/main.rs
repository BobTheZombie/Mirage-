#![no_std]
#![no_main]

extern crate mirage;

use mirage::arch::x86_64;
use mirage::kernel::{Kernel, MAX_PROCESSES, MESSAGE_DEPTH};
use mirage::subkernel::Credentials;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    x86_64::init_architecture();

    let mut kernel = Kernel::<MAX_PROCESSES, MESSAGE_DEPTH>::new();
    kernel.bootstrap();

    // Create an initial task with system privileges to kick-start the kernel.
    let init_creds = Credentials::system();
    let _ = kernel.spawn_initial_process(init_creds);

    loop {
        kernel.tick();
        x86_64::cpu_relax();
    }
}

