#![no_std]
#![no_main]

extern crate mirage;

use mirage::arch::x86_64::{self, boot::BootInfo};
use mirage::kernel::boot_status::{print_boot_complete_screen, BootStage, BootStatus};
#[cfg(not(feature = "full-boot"))]
use mirage::kernel::ipc::MessagePayload;
use mirage::kernel::{cpu, debug_shell, Kernel, MAX_PROCESSES, MESSAGE_DEPTH};
#[cfg(not(feature = "full-boot"))]
use mirage::subkernel::{Credentials, SecurityClass};
#[cfg(not(feature = "full-boot"))]
use mirage::supervisor::mock_service::{
    MockManifestCapability, MockManifestService, ECHO_IPC_ENDPOINT, ECHO_SERVICE_IMAGE,
    ECHO_SERVICE_MODULE_ID, IPC_ENDPOINT_CAPABILITY_OBJECT,
};
use mirage::supervisor::Supervisor;

#[no_mangle]
pub extern "Rust" fn kernel_main(boot_info: BootInfo) -> ! {
    let mut boot_status = BootStatus::new();
    mirage::kprintln!("Mirage kernel booting...");
    if !boot_info.limine_base_revision_supported() {
        mirage::kprintln!("unsupported Limine base revision");
        mirage::arch::x86_64::panic_halt();
    }
    mirage::kprintln!("architecture init starting");
    x86_64::init_architecture(&boot_info);
    boot_status.mark(BootStage::Architecture);

    let mut kernel = Kernel::<MAX_PROCESSES, MESSAGE_DEPTH>::new();
    mirage::kprintln!("kernel constructed");
    kernel.bootstrap_with_boot_info(&boot_info);
    boot_status.mark(BootStage::Memory);
    mirage::kprintln!("boot info applied");

    if cpu::MAX_CORES > 1 {
        kernel.bring_up_secondary_cores(cpu::MAX_CORES - 1);
    }

    let supervisor = Supervisor::new();
    mirage::kprintln!("supervisor created");

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
        boot_status.mark(BootStage::Supervisor);
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

        mirage::kprintln!("minimal supervisor bootstrap starting");
        let minimal_report = supervisor.bootstrap_minimal(&mut kernel);
        boot_status.mark(BootStage::Supervisor);
        mirage::kprintln!("minimal supervisor bootstrap complete");
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

    kernel.kernel_mtss_init();
    boot_status.mark(BootStage::Mtss);
    mirage::kprintln!("MTSS initialized");
    boot_status.mark(BootStage::IdleLoop);
    print_boot_complete_screen(&boot_status);
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
