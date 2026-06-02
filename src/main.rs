#![no_std]
#![no_main]

extern crate mirage;

use mirage::arch::x86_64::{self, boot::BootInfo};
use mirage::kernel::{cpu, Kernel, MAX_PROCESSES, MESSAGE_DEPTH};

#[no_mangle]
pub extern "Rust" fn kernel_main(boot_info: BootInfo) -> ! {
    x86_64::init_architecture(&boot_info);

    let mut kernel = Kernel::<MAX_PROCESSES, MESSAGE_DEPTH>::new();
    kernel.bootstrap_with_framebuffer(boot_info.framebuffer);

    if cpu::MAX_CORES > 1 {
        kernel.bring_up_secondary_cores(cpu::MAX_CORES - 1);
    }

    // Start L2 first, then L1-supervised device-facing daemons.
    let _ = kernel.bootstrap_services();

    loop {
        kernel.tick();
        x86_64::cpu_relax();
    }
}
