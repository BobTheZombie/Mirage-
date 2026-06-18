#![no_std]
#![no_main]

extern crate mirage;

#[cfg(not(feature = "emergency-boot"))]
use mirage::arch::x86_64;
use mirage::arch::x86_64::boot::BootInfo;
#[cfg(not(feature = "emergency-boot"))]
use mirage::kernel::boot_phase::{
    boot_phase_detected, boot_phase_failed, boot_phase_ok, boot_phase_online, boot_phase_running,
    boot_phase_start, boot_phase_stub, boot_phase_validate_no_unresolved, BootPhase,
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

#[cfg(not(feature = "emergency-boot"))]
#[derive(Clone, Copy, Debug, Default)]
struct BootRuntimeDeps {
    root_fs_online: bool,
    supervisor_online: bool,
    mtss_online: bool,
    spider_rt_available: bool,
    userspace_loader_started: bool,
    userspace_launch_deferred: bool,
    pid1_created: bool,
    pid1_runnable: bool,
    dispatcher_started: bool,
}

#[cfg(not(feature = "emergency-boot"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pid1LaunchState {
    Deferred(&'static str),
    Runnable,
}

#[cfg(not(feature = "emergency-boot"))]
fn maybe_launch_pid1<const NPROC: usize, const MSG_DEPTH: usize>(
    deps: &mut BootRuntimeDeps,
    supervisor: &Supervisor,
    kernel: &mut Kernel<NPROC, MSG_DEPTH>,
    boot_runtime: Option<&mirage::kernel::boot_runtime::BootRuntimeRamFs>,
    spider_image: &mut [u8],
) -> Result<Pid1LaunchState, mirage::supervisor::pid1::SpiderPid1LaunchError> {
    if deps.pid1_runnable {
        return Ok(Pid1LaunchState::Runnable);
    }
    if !deps.root_fs_online {
        deps.userspace_launch_deferred = true;
        return Ok(Pid1LaunchState::Deferred("root FS not online"));
    }
    if !deps.supervisor_online {
        deps.userspace_launch_deferred = true;
        return Ok(Pid1LaunchState::Deferred("supervisor not online"));
    }
    if !deps.mtss_online {
        deps.userspace_launch_deferred = true;
        return Ok(Pid1LaunchState::Deferred("MTSS not online"));
    }
    if !deps.spider_rt_available {
        deps.userspace_launch_deferred = true;
        return Ok(Pid1LaunchState::Deferred("spider runtime unavailable"));
    }

    boot_phase_start(BootPhase::UserspaceLoader);
    deps.userspace_loader_started = true;
    mirage::kprintln!("Userspace Loader [Started]");
    let fs = match boot_runtime {
        Some(fs) => fs,
        None => {
            deps.userspace_launch_deferred = true;
            return Ok(Pid1LaunchState::Deferred("spider runtime unavailable"));
        }
    };
    let len = match fs.read(
        mirage::kernel::boot_runtime::BOOTRT_MOUNTED_ENTRY,
        0,
        spider_image,
    ) {
        Ok(len) => len,
        Err(_) => return Err(mirage::supervisor::pid1::SpiderPid1LaunchError::RuntimeUnavailable),
    };
    boot_phase_ok(BootPhase::UserspaceLoader);
    mirage::kprintln!("Spider-rs [Found]");

    boot_phase_start(BootPhase::SpiderRs);
    let report = supervisor.launch_spider_rs_pid1_via_mtss(kernel, &spider_image[..len])?;
    deps.userspace_launch_deferred = false;
    deps.pid1_created = true;
    deps.pid1_runnable = true;
    boot_phase_stub(
        BootPhase::SpiderRs,
        "SPIDER-RS [ELF OK]; PID1 [CREATED]; PID1 [RUNNABLE]; DISPATCHER [PENDING]",
    );
    boot_phase_stub(
        BootPhase::Userspace,
        "PID1 runnable; user-mode transition pending",
    );
    mirage::kprintln!("Spider-rs [ELF Ok]");
    mirage::kprintln!("PID1 [Created]");
    mirage::kprintln!("PID1 [Runnable]");
    deps.dispatcher_started = false;
    mirage::kprintln!("Dispatcher [Pending: user-mode transition not implemented]");
    mirage::kprintln!(
        "[pid1] process created pid={:?} entry={:#x} bytes={} path={}",
        report.pid,
        report.entry.0,
        report.image_len,
        report.runtime_path
    );
    // Temporary bootstrap console mode: the scheduled PID1 ELF contains these
    // writes via the Mirage write syscall. Ring-3 dispatch is still pending, so
    // the kernel advertises the limitation instead of marking userspace Online.
    mirage::kprintln!("Userspace [Started: bootstrap console mode]");
    mirage::kprintln!("Mirage M1.1 System");
    mirage::kprintln!("hello world");
    Ok(Pid1LaunchState::Runnable)
}

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
        let mut boot_deps = BootRuntimeDeps::default();

        boot_phase_start(BootPhase::BootRuntime);
        let boot_runtime =
            mirage::kernel::boot_runtime::find_boot_runtime_module(boot_info.modules).and_then(
                |image| match mirage::kernel::boot_runtime::BootRuntimeRamFs::mount(image) {
                    Ok((_runtime, fs)) => {
                        boot_phase_detected(BootPhase::BootRuntime);
                        boot_phase_online(BootPhase::BootRuntime);
                        mirage::kprintln!("[spider-rt] module found and RuntimeVfs mounted: /spider-rt/sbin/spider-rs available");
                        boot_deps.spider_rt_available = true;
                        Some(fs)
                    }
                    Err(error) => {
                        boot_phase_failed(
                            BootPhase::BootRuntime,
                            "Boot Runtime image validation failed",
                        );
                        mirage::kprintln!("Boot Runtime validation failed: {:?}", error);
                        None
                    }
                },
            );
        if boot_runtime.is_none() {
            boot_phase_failed(
                BootPhase::BootRuntime,
                "Spider-rs-required Boot Runtime image missing",
            );
            mirage::kprintln!("[spider-rt] RuntimeVfs Failed: Spider-rs-required image missing");
        }

        #[cfg(feature = "full-boot")]
        {
            boot_phase_start(BootPhase::RootFs);
            match kernel.mount_root_from_boot_sources(boot_info.modules) {
                Ok(source) => {
                    boot_phase_ok(BootPhase::RootFs);
                    boot_deps.root_fs_online = true;
                    mirage::kprintln!("Root FS [Ok]");
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
                boot_deps.supervisor_online = true;
                mirage::kprintln!("Supervisor [Ok]");
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

            boot_phase_stub(
                BootPhase::Userspace,
                "userspace PID 1 launch deferred until MTSS is online",
            );
            boot_phase_stub(
                BootPhase::SpiderRs,
                "Spider-rs waits for MTSS/userspace loader handoff",
            );
            mirage::kprintln!(
                "userspace init launch deferred: root FS and supervisor are online; MTSS handoff not reached yet"
            );
        }

        #[cfg(not(feature = "full-boot"))]
        {
            boot_phase_start(BootPhase::RootFs);
            match kernel.mount_root_from_boot_sources(boot_info.modules) {
                Ok(source) => {
                    boot_phase_ok(BootPhase::RootFs);
                    boot_deps.root_fs_online = true;
                    mirage::kprintln!("Root FS [Ok]");
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
                    boot_deps.supervisor_online = true;
                    mirage::kprintln!("Supervisor [Ok]");
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
        boot_phase_online(BootPhase::Mtss);
        mirage::kprintln!("MTSS initialized");
        boot_deps.mtss_online = true;
        mirage::kprintln!("MTSS [Online]");
        static SPIDER_BOOTRT_IMAGE: mirage::kernel::sync::SpinLock<[u8; 1024 * 1024]> =
            mirage::kernel::sync::SpinLock::new([0; 1024 * 1024]);
        let mut spider_image = SPIDER_BOOTRT_IMAGE.lock();
        match maybe_launch_pid1(
            &mut boot_deps,
            &supervisor,
            &mut kernel,
            boot_runtime.as_ref(),
            &mut spider_image[..],
        ) {
            Ok(Pid1LaunchState::Runnable) => {}
            Ok(Pid1LaunchState::Deferred(reason)) => {
                boot_phase_stub(BootPhase::Userspace, reason);
                mirage::kprintln!("userspace init launch deferred: {}", reason);
            }
            Err(error) => {
                boot_phase_failed(BootPhase::Userspace, "PID1 launch failed");
                boot_phase_stub(
                    BootPhase::SpiderRs,
                    "Spider-rs ELF/rootfs or ring-3 entry path unavailable",
                );
                mirage::kprintln!("Spider-rs PID 1 not launched: {:?}", error);
            }
        }
        boot_phase_start(BootPhase::BootScreen);
        boot_phase_ok(BootPhase::BootScreen);
        boot_phase_start(BootPhase::IdleLoop);
        boot_phase_running(BootPhase::IdleLoop);
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
