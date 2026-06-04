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

    if cpu::MAX_CORES > 1 {
        kernel.bring_up_secondary_cores(cpu::MAX_CORES - 1);
    }

    let supervisor = Supervisor::new();

    #[cfg(feature = "full-boot")]
    {
        match kernel.mount_root_from_boot_sources(boot_info.modules) {
            Ok(source) => {
                mirage::kprintln!("root mount attempt succeeded: {:?}", source);
            }
            Err(error) => {
                mirage::kprintln!("root mount attempt failed: {:?}", error);
            }
        }

        // Start L2 first, then L1-supervised device-facing daemons.
        let service_report = supervisor.bootstrap_services(&mut kernel);
        if service_report.all_running() {
            mirage::kprintln!("supervisor initialization succeeded: full service manifest running");
        } else {
            mirage::kprintln!("supervisor initialization failed: full service manifest incomplete");
            let mut index = 0usize;
            while index < service_report.len() {
                if let Some(record) = service_report.record(index) {
                    if record.state != mirage::supervisor::StartupState::Running {
                        mirage::kprintln!(
                            "supervisor service '{}' did not reach Running: state={:?} failure={:?}",
                            record.descriptor.name,
                            record.state,
                            record.failure
                        );
                    }
                }
                index += 1;
            }
        }

        match kernel.bootstrap_userspace_init() {
            Ok(pid) => {
                mirage::kprintln!("userspace init attempt succeeded: pid={:?}", pid);
            }
            Err(error) => {
                mirage::kprintln!(
                    "userspace init attempt skipped/stubbed for minimal boot milestone: {:?}",
                    error
                );
            }
        }
    }

    #[cfg(not(feature = "full-boot"))]
    {
        mirage::kprintln!(
            "root mount attempt skipped: minimal boot milestone does not require QFS root yet"
        );

        let minimal_report = supervisor.bootstrap_minimal(&mut kernel);
        match minimal_report.failure {
            Some(error) => {
                mirage::kprintln!("supervisor initialization failed: {:?}", error);
            }
            None => {
                mirage::kprintln!(
                    "supervisor initialization succeeded: minimal registry entries={}",
                    minimal_report.len()
                );
            }
        }

        mirage::kprintln!(
            "userspace init attempt skipped: minimal boot milestone uses supervisor-only skeleton"
        );
    }

    mirage::kprintln!("Mirage reached idle loop");
    let mut observed_timer_ticks = x86_64::timer_ticks();
    loop {
        if x86_64::timer_tick_pending(&mut observed_timer_ticks) {
            kernel.tick();
        }
        x86_64::idle_halt();
    }
}
