#![no_std]
#![no_main]

extern crate mirage;

#[cfg(not(any(feature = "emergency-boot", feature = "seed-rs-qemu-emergency")))]
use mirage::arch::x86_64;
use mirage::arch::x86_64::boot::BootInfo;
#[cfg(not(any(feature = "emergency-boot", feature = "seed-rs-qemu-emergency")))]
use mirage::kernel::boot_screen::render_persistent_boot_screen;
use mirage::kernel::boot_status::{BootStage, BootState, BootStatus};
#[cfg(all(
    not(any(feature = "emergency-boot", feature = "seed-rs-qemu-emergency")),
    not(feature = "full-boot")
))]
use mirage::kernel::ipc::MessagePayload;
#[cfg(not(any(feature = "emergency-boot", feature = "seed-rs-qemu-emergency")))]
use mirage::kernel::{cpu, debug_shell, Kernel, MAX_PROCESSES, MESSAGE_DEPTH};
#[cfg(all(
    not(any(feature = "emergency-boot", feature = "seed-rs-qemu-emergency")),
    not(feature = "full-boot")
))]
use mirage::subkernel::{Credentials, SecurityClass};
#[cfg(all(
    not(any(feature = "emergency-boot", feature = "seed-rs-qemu-emergency")),
    not(feature = "full-boot")
))]
use mirage::supervisor::mock_service::{
    MockManifestCapability, MockManifestService, ECHO_IPC_ENDPOINT, ECHO_SERVICE_IMAGE,
    ECHO_SERVICE_MODULE_ID, IPC_ENDPOINT_CAPABILITY_OBJECT,
};
#[cfg(not(any(feature = "emergency-boot", feature = "seed-rs-qemu-emergency")))]
use mirage::supervisor::Supervisor;

#[no_mangle]
pub extern "Rust" fn kernel_main(boot_info: BootInfo) -> ! {
    #[cfg(not(feature = "seed-rs-qemu-emergency"))]
    unsafe {
        mirage::arch::x86_64::early_debug::boot_marker(7);
    }

    #[cfg(feature = "seed-rs-qemu-emergency")]
    {
        let _ = &boot_info;
        unsafe {
            mirage::arch::x86_64::seed_rs::seed_com1_write_str(
                "Mirage seed-rs QEMU emergency boot reached idle loop\r\n",
            );
        }
        mirage::arch::x86_64::panic_halt();
    }

    #[cfg(feature = "emergency-boot")]
    {
        let _ = &boot_info;
        unsafe {
            mirage::arch::x86_64::early_debug::com1_write_str(
                "Mirage emergency boot reached idle loop",
            );
        }
        mirage::arch::x86_64::panic_halt();
    }

    #[cfg(not(any(feature = "emergency-boot", feature = "seed-rs-qemu-emergency")))]
    {
        mirage::kprintln!("Mirage kernel booting...");
        let mut boot_status = BootStatus::new();
        if !boot_info.limine_base_revision_supported() {
            mirage::kprintln!("unsupported Limine base revision");
            mirage::arch::x86_64::panic_halt();
        }
        mirage::kprintln!("architecture init starting");
        unsafe {
            mirage::arch::x86_64::early_debug::boot_marker(8);
        }
        x86_64::init_architecture(&boot_info, &mut boot_status);
        unsafe {
            mirage::arch::x86_64::early_debug::boot_marker(9);
        }
        let mut kernel = Kernel::<MAX_PROCESSES, MESSAGE_DEPTH>::new();
        mirage::kprintln!("kernel constructed");
        kernel.bootstrap_with_boot_info(&boot_info);
        boot_status.set_stage(BootStage::Memory);
        // Memory, paging, and heap remain milestone-pending until ownership and allocator
        // milestones make those subsystems official boot-screen statuses.
        mirage::kprintln!("boot info applied");
        render_persistent_boot_screen(&boot_status);

        if cpu::MAX_CORES > 1 {
            kernel.bring_up_secondary_cores(cpu::MAX_CORES - 1);
        }

        let supervisor = Supervisor::new();
        mirage::kprintln!("supervisor created");

        #[cfg(feature = "full-boot")]
        {
            boot_status.set_stage(BootStage::RootFs);
            match kernel.mount_root_from_boot_sources(boot_info.modules) {
                Ok(source) => {
                    boot_status.root_fs = BootState::Ok;
                    mirage::kprintln!("root mount attempt succeeded: {:?}", source);
                }
                Err(error) => {
                    boot_status.root_fs = BootState::Failed;
                    mirage::kprintln!("root mount attempt failed: {:?}", error);
                }
            }
            render_persistent_boot_screen(&boot_status);

            // Start L2 first, then L1-supervised device-facing daemons.
            boot_status.set_stage(BootStage::Supervisor);
            let service_report = supervisor.bootstrap_services(&mut kernel);
            if service_report.all_running() {
                boot_status.supervisor = BootState::Ok;
                mirage::kprintln!(
                    "supervisor initialization succeeded: full service manifest running"
                );
            } else {
                boot_status.supervisor = BootState::Failed;
                mirage::kprintln!(
                    "supervisor initialization failed: full service manifest incomplete"
                );
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

            render_persistent_boot_screen(&boot_status);
            boot_status.set_stage(BootStage::Userspace);
            match kernel.bootstrap_userspace_init() {
                Ok(pid) => {
                    boot_status.userspace = BootState::Ok;
                    mirage::kprintln!("userspace init attempt succeeded: pid={:?}", pid);
                }
                Err(error) => {
                    boot_status.userspace = BootState::Stub;
                    mirage::kprintln!(
                        "userspace init attempt skipped/stubbed for minimal boot milestone: {:?}",
                        error
                    );
                }
            }
            render_persistent_boot_screen(&boot_status);
        }

        #[cfg(not(feature = "full-boot"))]
        {
            boot_status.set_stage(BootStage::RootFs);
            match kernel.mount_root_from_boot_sources(boot_info.modules) {
                Ok(source) => {
                    boot_status.root_fs = BootState::Ok;
                    mirage::kprintln!("root mount attempt succeeded: {:?}", source);
                }
                Err(error) => {
                    boot_status.root_fs = BootState::Failed;
                    mirage::kprintln!("root mount attempt failed: {:?}", error);
                }
            }
            render_persistent_boot_screen(&boot_status);

            boot_status.set_stage(BootStage::Supervisor);
            mirage::kprintln!("minimal supervisor bootstrap starting");
            let minimal_report = supervisor.bootstrap_minimal(&mut kernel);
            mirage::kprintln!("minimal supervisor bootstrap complete");
            match minimal_report.failure {
                Some(error) => {
                    boot_status.supervisor = BootState::Failed;
                    mirage::kprintln!("supervisor initialization failed: {:?}", error);
                }
                None => {
                    boot_status.supervisor = BootState::Ok;
                    mirage::kprintln!(
                        "supervisor initialization succeeded: minimal registry entries={}",
                        minimal_report.len()
                    );
                }
            }

            render_persistent_boot_screen(&boot_status);
            boot_status.set_stage(BootStage::Userspace);
            boot_status.userspace = BootState::Stub;
            mirage::kprintln!(
            "userspace init attempt skipped: minimal boot milestone uses supervisor-only skeleton"
        );
            render_persistent_boot_screen(&boot_status);

            mirage::kprintln!("loading boot manifest");
            // Temporary compiled-in manifest fixture: replace this with Limine module
            // discovery or QFS-backed manifest lookup once those boot sources are
            // available during the non-full-boot smoke path.
            let echo_rights = ["SEND", "RECEIVE"];
            let echo_capabilities = [MockManifestCapability {
                object: IPC_ENDPOINT_CAPABILITY_OBJECT,
                endpoint: Some(ECHO_IPC_ENDPOINT),
                rights: &echo_rights,
            }];
            let echo_service = MockManifestService {
                module_id: ECHO_SERVICE_MODULE_ID,
                image: ECHO_SERVICE_IMAGE,
                restart_always: true,
                capabilities: &echo_capabilities,
            };
            mirage::kprintln!("boot manifest validated");

            mirage::kprintln!("launching service: echo-service");
            match supervisor.launch_mock_manifest_service(&mut kernel, echo_service) {
                Ok(echo_report) => {
                    mirage::kprintln!("service running: echo-service");
                    match kernel.spawn_initial_process(Credentials::system()) {
                        Ok(caller) => {
                            let payload = MessagePayload::from_slice(
                                SecurityClass::Internal,
                                b"mirage echo smoke",
                            );
                            match supervisor.dispatch_echo_request(
                                &mut kernel,
                                &echo_report,
                                caller,
                                payload,
                            ) {
                                Ok(response) if response == payload => {
                                    mirage::kprintln!("echo-service IPC check passed");
                                }
                                Ok(_) => {
                                    mirage::kprintln!(
                                        "echo-service IPC check failed: response payload mismatch"
                                    );
                                }
                                Err(error) => {
                                    mirage::kprintln!("echo-service IPC check failed: {:?}", error);
                                }
                            }
                        }
                        Err(error) => {
                            mirage::kprintln!(
                                "echo-service IPC check failed: caller spawn error: {:?}",
                                error
                            );
                        }
                    }
                }
                Err(error) => {
                    mirage::kprintln!("service launch failed: echo-service: {:?}", error);
                }
            }
        }

        boot_status.set_stage(BootStage::Mtss);
        kernel.kernel_mtss_init();
        boot_status.mtss = BootState::Ok;
        mirage::kprintln!("MTSS initialized");
        if boot_status.memory == BootState::Pending {
            boot_status.set_stage(BootStage::Memory);
        } else {
            boot_status.set_stage(BootStage::IdleLoop);
        }
        render_persistent_boot_screen(&boot_status);
        let mut observed_timer_ticks = x86_64::timer_ticks();
        loop {
            if x86_64::poll_debug_shell_hotkey() {
                debug_shell::enter_early_debug_shell(&mut kernel, &boot_status);
            }
            if x86_64::timer_tick_pending(&mut observed_timer_ticks) {
                kernel.tick();
            }
            x86_64::idle_halt();
        }
    }
}
