#![no_std]
#![no_main]

extern crate mirage;

#[cfg(not(feature = "emergency-boot"))]
use mirage::arch::x86_64;
use mirage::arch::x86_64::boot::BootInfo;
#[cfg(all(not(feature = "emergency-boot"), feature = "full-boot"))]
use mirage::kernel::boot_phase::boot_phase_online;
#[cfg(not(feature = "emergency-boot"))]
use mirage::kernel::boot_phase::{
    boot_phase_failed, boot_phase_ok, boot_phase_start, boot_phase_stub,
    boot_phase_validate_no_unresolved, BootPhase,
};
#[cfg(all(not(feature = "emergency-boot"), not(feature = "full-boot")))]
use mirage::kernel::ipc::MessagePayload;
#[cfg(not(feature = "emergency-boot"))]
use mirage::kernel::{cpu, debug_shell, Kernel, MAX_PROCESSES, MESSAGE_DEPTH};
#[cfg(all(not(feature = "emergency-boot"), not(feature = "full-boot")))]
use mirage::subkernel::{Credentials, SecurityClass};
#[cfg(all(not(feature = "emergency-boot"), not(feature = "full-boot")))]
use mirage::supervisor::mock_service::{
    MockManifestCapability, MockManifestService, ECHO_IPC_ENDPOINT, ECHO_SERVICE_IMAGE,
    ECHO_SERVICE_MODULE_ID, IPC_ENDPOINT_CAPABILITY_OBJECT,
};
#[cfg(not(feature = "emergency-boot"))]
use mirage::supervisor::Supervisor;

#[no_mangle]
pub extern "Rust" fn kernel_main(boot_info: BootInfo) -> ! {
    #[cfg(not(feature = "emergency-boot"))]
    boot_phase_start(BootPhase::KernelMain);

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

    #[cfg(not(feature = "emergency-boot"))]
    {
        mirage::kprintln!("Mirage kernel booting...");
        if !boot_info.limine_base_revision_supported() {
            boot_phase_failed(BootPhase::BootInfo, "unsupported Limine base revision");
            mirage::kprintln!("unsupported Limine base revision");
            mirage::arch::x86_64::panic_halt();
        }
        boot_phase_ok(BootPhase::KernelMain);
        mirage::kprintln!("architecture init starting");
        boot_phase_start(BootPhase::Architecture);
        x86_64::init_architecture(&boot_info);
        boot_phase_ok(BootPhase::Architecture);
        boot_phase_start(BootPhase::KernelConstructed);
        let mut kernel = Kernel::<MAX_PROCESSES, MESSAGE_DEPTH>::new();
        boot_phase_ok(BootPhase::KernelConstructed);
        mirage::kprintln!("kernel constructed");
        boot_phase_start(BootPhase::BootInfoApplied);
        kernel.bootstrap_with_boot_info(&boot_info);
        boot_phase_ok(BootPhase::BootInfoApplied);
        mirage::kprintln!("boot info applied");

        if cpu::MAX_CORES > 1 {
            kernel.bring_up_secondary_cores(cpu::MAX_CORES - 1);
        }

        boot_phase_start(BootPhase::SupervisorCreated);
        let supervisor = Supervisor::new();
        boot_phase_ok(BootPhase::SupervisorCreated);
        mirage::kprintln!("supervisor created");

        #[cfg(feature = "full-boot")]
        {
            boot_phase_start(BootPhase::RootFs);
            match kernel.mount_root_from_boot_sources(boot_info.modules) {
                Ok(source) => {
                    boot_phase_ok(BootPhase::RootFs);
                    mirage::kprintln!("root mount attempt succeeded: {:?}", source);
                }
                Err(error) => {
                    boot_phase_failed(BootPhase::RootFs, "root mount failed");
                    mirage::kprintln!("root mount attempt failed: {:?}", error);
                }
            }
            // Start L2 first, then L1-supervised device-facing daemons.
            boot_phase_start(BootPhase::Supervisor);
            let service_report = supervisor.bootstrap_services(&mut kernel);
            if service_report.all_running() {
                boot_phase_ok(BootPhase::Supervisor);
                mirage::kprintln!(
                    "supervisor initialization succeeded: full service manifest running"
                );
            } else {
                boot_phase_failed(BootPhase::Supervisor, "full service manifest incomplete");
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

            boot_phase_start(BootPhase::Userspace);
            match kernel.bootstrap_userspace_init() {
                Ok((pid, init_path)) => {
                    boot_phase_ok(BootPhase::Userspace);
                    if init_path == "/sbin/spider-rs" {
                        boot_phase_online(BootPhase::SpiderRs);
                    } else {
                        boot_phase_stub(BootPhase::SpiderRs, "Spider-rs image not selected");
                    }
                    mirage::kprintln!(
                        "userspace init attempt succeeded: path={} pid={:?}",
                        init_path,
                        pid
                    );
                }
                Err(error) => {
                    boot_phase_stub(
                        BootPhase::Userspace,
                        "userspace init unavailable in milestone",
                    );
                    boot_phase_stub(
                        BootPhase::SpiderRs,
                        "Spider-rs waits for userspace exec ABI",
                    );
                    mirage::kprintln!(
                        "userspace init attempt skipped/stubbed for minimal boot milestone: {:?}",
                        error
                    );
                }
            }
        }

        #[cfg(not(feature = "full-boot"))]
        {
            boot_phase_start(BootPhase::RootFs);
            match kernel.mount_root_from_boot_sources(boot_info.modules) {
                Ok(source) => {
                    boot_phase_ok(BootPhase::RootFs);
                    mirage::kprintln!("root mount attempt succeeded: {:?}", source);
                }
                Err(error) => {
                    boot_phase_failed(BootPhase::RootFs, "root mount failed");
                    mirage::kprintln!("root mount attempt failed: {:?}", error);
                }
            }
            boot_phase_start(BootPhase::Supervisor);
            mirage::kprintln!("minimal supervisor bootstrap starting");
            let minimal_report = supervisor.bootstrap_minimal(&mut kernel);
            mirage::kprintln!("minimal supervisor bootstrap complete");
            match minimal_report.failure {
                Some(error) => {
                    boot_phase_failed(BootPhase::Supervisor, "minimal supervisor bootstrap failed");
                    mirage::kprintln!("supervisor initialization failed: {:?}", error);
                }
                None => {
                    boot_phase_ok(BootPhase::Supervisor);
                    mirage::kprintln!(
                        "supervisor initialization succeeded: minimal registry entries={}",
                        minimal_report.len()
                    );
                }
            }

            boot_phase_stub(
                BootPhase::Userspace,
                "minimal boot milestone uses supervisor-only skeleton",
            );
            boot_phase_stub(
                BootPhase::SpiderRs,
                "Spider-rs pending real userspace init launch",
            );
            mirage::kprintln!(
            "userspace init attempt skipped: minimal boot milestone uses supervisor-only skeleton"
        );

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

        boot_phase_start(BootPhase::Mtss);
        kernel.kernel_mtss_init();
        boot_phase_ok(BootPhase::Mtss);
        mirage::kprintln!("MTSS initialized");
        boot_phase_start(BootPhase::BootScreen);
        boot_phase_ok(BootPhase::BootScreen);
        boot_phase_start(BootPhase::IdleLoop);
        boot_phase_validate_no_unresolved();
        let mut observed_timer_ticks = x86_64::timer_ticks();
        loop {
            if x86_64::poll_debug_shell_hotkey() {
                debug_shell::enter_early_debug_shell(&mut kernel);
            }
            if x86_64::timer_tick_pending(&mut observed_timer_ticks) {
                kernel.tick();
            }
            x86_64::idle_halt();
        }
    }
}
