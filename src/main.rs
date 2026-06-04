#![no_std]
#![no_main]

extern crate mirage;

use mirage::arch::x86_64::{self, boot::BootInfo};
use mirage::kernel::{cpu, Kernel, MAX_PROCESSES, MESSAGE_DEPTH};
use mirage::supervisor::Supervisor;

#[no_mangle]
pub extern "Rust" fn kernel_main(boot_info: BootInfo) -> ! {
    mirage::kprintln!("Mirage kernel booting...");
    x86_64::init_architecture(&boot_info);

    let mut kernel = Kernel::<MAX_PROCESSES, MESSAGE_DEPTH>::new();
    kernel.bootstrap_with_boot_info(&boot_info);
    let _ = kernel.mount_root_from_boot_sources(boot_info.modules);

    if cpu::MAX_CORES > 1 {
        kernel.bring_up_secondary_cores(cpu::MAX_CORES - 1);
    }

    // Start L2 first, then L1-supervised device-facing daemons.
    let supervisor = Supervisor::new();
    let _ = supervisor.bootstrap_services(&mut kernel);
    mirage::kprintln!("supervisor initialized");

    let _ = kernel.bootstrap_userspace_init();

    mirage::kprintln!("Mirage reached idle loop");
    let mut observed_timer_ticks = x86_64::timer_ticks();
    loop {
        if x86_64::timer_tick_pending(&mut observed_timer_ticks) {
            kernel.tick();
        }
        x86_64::idle_halt();
    }
}
