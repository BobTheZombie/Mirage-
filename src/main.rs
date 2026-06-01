#![no_std]
#![no_main]

extern crate mirage;

use mirage::arch::x86_64::{self, boot::BootInfo};
use mirage::kernel::{cpu, Kernel, MAX_PROCESSES, MESSAGE_DEPTH};
use mirage::subkernel::Credentials;

#[no_mangle]
pub extern "Rust" fn kernel_main(boot_info: BootInfo) -> ! {
    x86_64::init_architecture(&boot_info);

    let mut kernel = Kernel::<MAX_PROCESSES, MESSAGE_DEPTH>::new();
    kernel.bootstrap();

    if cpu::MAX_CORES > 1 {
        kernel.bring_up_secondary_cores(cpu::MAX_CORES - 1);
    }

    // Create an initial task with system privileges to kick-start the kernel.
    let init_creds = Credentials::system();
    let _ = kernel.spawn_initial_process(init_creds);

    loop {
        kernel.tick();
        x86_64::cpu_relax();
    }
}
