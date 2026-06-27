#![no_std]
#![no_main]

extern crate mirage;

#[cfg(not(feature = "emergency-boot"))]
use core::mem::MaybeUninit;

#[cfg(not(feature = "emergency-boot"))]
use mirage::arch::x86_64;
use mirage::arch::x86_64::boot::BootInfo;
#[cfg(not(feature = "emergency-boot"))]
use mirage::kernel::boot_phase::{
    boot_phase_failed, boot_phase_found, boot_phase_ok, boot_phase_online, boot_phase_pending,
    boot_phase_running, boot_phase_skipped, boot_phase_start, boot_phase_stub,
    boot_phase_validate_no_unresolved, BootPhase,
};
#[cfg(all(not(feature = "emergency-boot"), not(feature = "full-boot")))]
use mirage::kernel::ipc::MessagePayload;
#[cfg(not(feature = "emergency-boot"))]
use mirage::kernel::kso::{
    maybe_retry_pid1_handoff_after_mtss_change, BootContinueResult, BootRuntimeDeps, KsoContext,
};
#[cfg(not(feature = "emergency-boot"))]
use mirage::kernel::{cpu, debug_shell, Kernel, MAX_PROCESSES, MESSAGE_DEPTH};
#[cfg(all(not(feature = "emergency-boot"), not(feature = "full-boot")))]
use mirage::subkernel::{Credentials, SecurityClass};
#[cfg(all(not(feature = "emergency-boot"), not(feature = "full-boot")))]
use mirage::supervisor::mock_service::{
    MockManifestCapability, MockManifestService, ECHO_IPC_ENDPOINT, ECHO_SERVICE_IMAGE,
    ECHO_SERVICE_MODULE_ID, IPC_ENDPOINT_CAPABILITY_OBJECT,
};
use mirage::supervisor::Supervisor;

#[cfg(not(feature = "emergency-boot"))]
fn bootflow(seq: u8, phase: &'static str, status: &'static str) {
    mirage::kprintln!("[bootflow {}] phase={} {}", seq, phase, status);
}

#[cfg(not(feature = "emergency-boot"))]
static mut BOOT_KERNEL_STORAGE: MaybeUninit<Kernel<MAX_PROCESSES, MESSAGE_DEPTH>> =
    MaybeUninit::uninit();

#[cfg(not(feature = "emergency-boot"))]
fn boot_kernel_constructed_phase() -> &'static mut Kernel<MAX_PROCESSES, MESSAGE_DEPTH> {
    bootflow(2, "kernel_constructed", "enter");
    mirage::kprintln!("[bootflow 2.1] kernel_constructed: set milestone phase enter");
    boot_phase_start(BootPhase::KernelConstructed);
    let kernel = unsafe {
        let storage = core::ptr::addr_of_mut!(BOOT_KERNEL_STORAGE);
        (*storage).write(Kernel::<MAX_PROCESSES, MESSAGE_DEPTH>::new())
    };
    boot_phase_ok(BootPhase::KernelConstructed);
    mirage::kprintln!("[bootflow 2.1] kernel_constructed: set milestone phase ok");
    mirage::kprintln!("[bootflow 2.2] kernel_constructed: render UI skipped (continuation edge)");
    mirage::kprintln!("[bootflow 2.3] kernel_constructed: debug poll enter");
    if x86_64::poll_debug_shell_hotkey() {
        debug_shell::enter_early_debug_shell(kernel);
    }
    mirage::kprintln!("[bootflow 2.3] kernel_constructed: debug poll ok");
    mirage::kprintln!("[bootflow 2.4] kernel_constructed: return/advance ok");
    bootflow(2, "kernel_constructed", "ok");
    kernel
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
        bootflow(1, "architecture_init", "ok");
        let kernel = boot_kernel_constructed_phase();
        mirage::kprintln!("kernel constructed");
        bootflow(3, "boot_info_applied", "enter");
        #[cfg(any(feature = "bootdiag-serial", feature = "bootdiag-verbose"))]
        mirage::kprintln!("[bootdiag] boot info apply starting");
        boot_phase_start(BootPhase::BootInfoApplied);
        if let Err(error) = kernel.bootstrap_with_boot_info(&boot_info) {
            boot_phase_failed(BootPhase::BootInfoApplied, "kernel boot-info apply failed");
            bootflow(
                3,
                "boot_info_applied",
                "failed: kernel boot-info apply failed",
            );
            mirage::kprintln!("boot info apply failed: {:?}", error);
            mirage::arch::x86_64::panic_halt();
        }
        #[cfg(any(feature = "bootdiag-serial", feature = "bootdiag-verbose"))]
        mirage::kprintln!("[bootdiag] boot info apply returned");
        boot_phase_ok(BootPhase::BootInfoApplied);
        bootflow(3, "boot_info_applied", "ok");
        mirage::kprintln!("boot info applied");

        if cpu::MAX_CORES > 1 {
            kernel.bring_up_secondary_cores(cpu::MAX_CORES - 1);
        }

        #[cfg(any(feature = "bootdiag-serial", feature = "bootdiag-verbose"))]
        mirage::kprintln!("[bootdiag] supervisor creation starting");
        bootflow(4, "supervisor_create", "enter");
        boot_phase_start(BootPhase::SupervisorCreated);
        let supervisor = Supervisor::new();
        #[cfg(any(feature = "bootdiag-serial", feature = "bootdiag-verbose"))]
        mirage::kprintln!("[bootdiag] supervisor creation returned");
        boot_phase_ok(BootPhase::SupervisorCreated);
        bootflow(4, "supervisor_create", "ok");
        mirage::kprintln!("supervisor created");
        let mut boot_deps = BootRuntimeDeps::default();

        bootflow(5, "boot_runtime_validation", "enter");
        boot_phase_start(BootPhase::BootRuntime);
        let boot_runtime =
            mirage::kernel::boot_runtime::find_boot_runtime_module(boot_info.modules).and_then(
                |image| match mirage::kernel::boot_runtime::BootRuntimeRamFs::mount(image) {
                    Ok((_runtime, fs)) => {
                        boot_phase_ok(BootPhase::BootRuntime);
                        bootflow(5, "boot_runtime_validation", "ok");
                        bootflow(6, "runtime_vfs_mount", "ok");
                        boot_phase_found(BootPhase::SpiderRs);
                        boot_phase_found(BootPhase::SystemDispatcher);
                        mirage::kprintln!("BOOT RUNTIME [OK]");
                        mirage::kprintln!("SPIDER-RS IMAGE [FOUND]");
                        mirage::kprintln!("SPIDER-RSD IMAGE [FOUND]");
                        mirage::kprintln!("[spider-rt] module found and RuntimeVfs mounted: /spider-rt/sbin/spider-rs and /spider-rt/sbin/spider-rsd available");
                        boot_deps.spider_rt_available = true;
                        Some(fs)
                    }
                    Err(error) => {
                        boot_phase_failed(
                            BootPhase::BootRuntime,
                            "Boot Runtime image validation failed",
                        );
                        bootflow(5, "boot_runtime_validation", "failed: validation failed");
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
            bootflow(5, "boot_runtime_validation", "failed: image missing");
            boot_phase_failed(BootPhase::SpiderRs, "missing from Boot Runtime image");
            boot_phase_skipped(BootPhase::RootFs, "boot runtime invalid");
            boot_phase_skipped(BootPhase::UserspaceLoader, "boot runtime invalid");
            mirage::kprintln!(
                "[BOOTDIAG] ERROR BOOT RUNTIME: BOOT RUNTIME IMAGE VALIDATION FAILED"
            );
            mirage::kprintln!("[BOOTDIAG] ERROR BOOT RUNTIME: /spider-rt/sbin/spider-rs missing");
            mirage::kprintln!("[spider-rt] RuntimeVfs Failed: Spider-rs-required image missing");
        }

        #[cfg(feature = "full-boot")]
        {
            #[cfg(any(feature = "bootdiag-serial", feature = "bootdiag-verbose"))]
            mirage::kprintln!("[bootdiag] rootfs mount starting");
            bootflow(7, "rootfs_mount", "enter");
            if boot_runtime.is_some() {
                boot_phase_start(BootPhase::RootFs);
                match kernel.mount_root_from_boot_sources(boot_info.modules) {
                    Ok(source) => {
                        boot_phase_ok(BootPhase::RootFs);
                        bootflow(7, "rootfs_mount", "ok");
                        boot_deps.root_fs_resolved = true;
                        boot_deps.root_fs_online = true;
                        mirage::kprintln!("ROOT FS [OK]");
                        mirage::kprintln!("root mount attempt succeeded: {:?}", source);
                    }
                    Err(error) => {
                        boot_deps.root_fs_resolved = true;
                        boot_phase_failed(BootPhase::RootFs, "no root source configured");
                        bootflow(7, "rootfs_mount", "failed: no root source configured");
                        mirage::kprintln!("ROOT FS [FAILED: no root source configured]");
                        mirage::kprintln!("root mount attempt failed: {:?}", error);
                    }
                }
            } else {
                boot_deps.root_fs_resolved = true;
                boot_phase_skipped(BootPhase::RootFs, "boot runtime invalid");
                bootflow(7, "rootfs_mount", "failed: boot runtime invalid");
                mirage::kprintln!("ROOT FS [SKIPPED: boot runtime invalid]");
            }
            // Start L2 first, then L1-supervised device-facing daemons.
            #[cfg(any(feature = "bootdiag-serial", feature = "bootdiag-verbose"))]
            mirage::kprintln!("[bootdiag] supervisor bootstrap starting");
            bootflow(8, "supervisor_init", "enter");
            boot_phase_start(BootPhase::Supervisor);
            let service_report = supervisor.bootstrap_services(kernel);
            if service_report.all_running() {
                boot_phase_ok(BootPhase::Supervisor);
                bootflow(8, "supervisor_init", "ok");
                boot_deps.supervisor_online = true;
                mirage::kprintln!("Supervisor [Ok]");
                mirage::kprintln!(
                    "supervisor initialization succeeded: full service manifest running"
                );
            } else {
                boot_phase_failed(BootPhase::Supervisor, "full service manifest incomplete");
                bootflow(
                    8,
                    "supervisor_init",
                    "failed: full service manifest incomplete",
                );
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

            boot_phase_stub(BootPhase::Userspace, "PENDING: MTSS scheduler not ready");
            boot_phase_stub(BootPhase::SpiderRs, "PENDING: MTSS scheduler not ready");
            mirage::kprintln!("PID1 HANDOFF [PENDING: MTSS scheduler not ready]");
        }

        #[cfg(not(feature = "full-boot"))]
        {
            #[cfg(any(feature = "bootdiag-serial", feature = "bootdiag-verbose"))]
            mirage::kprintln!("[bootdiag] rootfs mount starting");
            bootflow(7, "rootfs_mount", "enter");
            if boot_runtime.is_some() {
                boot_phase_start(BootPhase::RootFs);
                match kernel.mount_root_from_boot_sources(boot_info.modules) {
                    Ok(source) => {
                        boot_phase_ok(BootPhase::RootFs);
                        bootflow(7, "rootfs_mount", "ok");
                        boot_deps.root_fs_resolved = true;
                        boot_deps.root_fs_online = true;
                        mirage::kprintln!("ROOT FS [OK]");
                        mirage::kprintln!("root mount attempt succeeded: {:?}", source);
                    }
                    Err(error) => {
                        boot_deps.root_fs_resolved = true;
                        boot_phase_failed(BootPhase::RootFs, "no root source configured");
                        bootflow(7, "rootfs_mount", "failed: no root source configured");
                        mirage::kprintln!("ROOT FS [FAILED: no root source configured]");
                        mirage::kprintln!("root mount attempt failed: {:?}", error);
                    }
                }
            } else {
                boot_deps.root_fs_resolved = true;
                boot_phase_skipped(BootPhase::RootFs, "boot runtime invalid");
                bootflow(7, "rootfs_mount", "failed: boot runtime invalid");
                mirage::kprintln!("ROOT FS [SKIPPED: boot runtime invalid]");
            }
            #[cfg(any(feature = "bootdiag-serial", feature = "bootdiag-verbose"))]
            mirage::kprintln!("[bootdiag] supervisor bootstrap starting");
            bootflow(8, "supervisor_init", "enter");
            boot_phase_start(BootPhase::Supervisor);
            mirage::kprintln!("minimal supervisor bootstrap starting");
            let minimal_report = supervisor.bootstrap_minimal(kernel);
            mirage::kprintln!("minimal supervisor bootstrap complete");
            match minimal_report.failure {
                Some(error) => {
                    boot_phase_failed(BootPhase::Supervisor, "minimal supervisor bootstrap failed");
                    bootflow(
                        8,
                        "supervisor_init",
                        "failed: minimal supervisor bootstrap failed",
                    );
                    mirage::kprintln!("supervisor initialization failed: {:?}", error);
                }
                None => {
                    boot_phase_ok(BootPhase::Supervisor);
                    bootflow(8, "supervisor_init", "ok");
                    boot_deps.supervisor_online = true;
                    mirage::kprintln!("Supervisor [Ok]");
                    mirage::kprintln!(
                        "supervisor initialization succeeded: minimal registry entries={}",
                        minimal_report.len()
                    );
                }
            }

            boot_phase_stub(BootPhase::Userspace, "PENDING: MTSS scheduler not ready");
            boot_phase_stub(BootPhase::SpiderRs, "PENDING: MTSS scheduler not ready");
            mirage::kprintln!("PID1 HANDOFF [PENDING: MTSS scheduler not ready]");

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
            match supervisor.launch_mock_manifest_service(kernel, echo_service) {
                Ok(echo_report) => {
                    mirage::kprintln!("service running: echo-service");
                    match kernel.spawn_initial_process(Credentials::system()) {
                        Ok(caller) => {
                            let payload = MessagePayload::from_slice(
                                SecurityClass::Internal,
                                b"mirage echo smoke",
                            );
                            match supervisor.dispatch_echo_request(
                                kernel,
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

        #[cfg(any(feature = "bootdiag-serial", feature = "bootdiag-verbose"))]
        mirage::kprintln!("[bootdiag] MTSS init starting");
        bootflow(9, "mtss_init", "enter");
        boot_phase_start(BootPhase::Mtss);
        match kernel.kernel_mtss_init() {
            Ok(report) => {
                #[cfg(any(feature = "bootdiag-serial", feature = "bootdiag-verbose"))]
                mirage::kprintln!("[bootdiag] MTSS init returned");
                mirage::kprintln!(
                    "MTSS CORE [{}]",
                    if report.core_ready {
                        "READY"
                    } else {
                        "PENDING"
                    }
                );
                mirage::kprintln!(
                    "MTSS SCHEDULER [{}]",
                    if report.scheduler_ready {
                        "READY"
                    } else {
                        "PENDING"
                    }
                );
                mirage::kprintln!(
                    "MTSS TIMER [{}]",
                    if report.timer_ready {
                        "READY"
                    } else {
                        "PENDING"
                    }
                );
                mirage::kprintln!(
                    "MTSS PREEMPTION [{}]",
                    if report.preemption_ready {
                        "READY"
                    } else {
                        "PENDING"
                    }
                );
                boot_deps.mtss.core_ready = report.core_ready;
                boot_deps.mtss.scheduler_ready = report.scheduler_ready;
                boot_deps.mtss.timer_ready = report.timer_ready;
                boot_deps.mtss.preemption_ready = report.preemption_ready;
                boot_deps.mtss.idle_ready = report.idle_ready;
                boot_deps.mtss.task_creation_api_ready = report.api_ready;
                boot_deps.mtss.mark_runnable_api_ready = report.api_ready;

                if boot_deps.mtss.fully_online() {
                    boot_phase_online(BootPhase::Mtss);
                    bootflow(9, "mtss_init", "ok");
                    mirage::kprintln!("MTSS [ ONLINE ]");
                } else if !report.timer_ready || !report.preemption_ready {
                    boot_phase_pending(BootPhase::Mtss, "timer/preemption backend pending");
                    bootflow(9, "mtss_init", "ok");
                    mirage::kprintln!("MTSS [DEGRADED: timer/preemption backend pending]");
                } else {
                    boot_phase_pending(BootPhase::Mtss, "required component pending");
                    bootflow(9, "mtss_init", "failed: required component pending");
                    mirage::kprintln!("MTSS [PENDING: required component pending]");
                }
            }
            Err(error) => {
                #[cfg(any(feature = "bootdiag-serial", feature = "bootdiag-verbose"))]
                mirage::kprintln!("[bootdiag] MTSS init failed");
                boot_deps.mtss.failed = true;
                boot_phase_failed(BootPhase::Mtss, "MTSS initialization failed");
                bootflow(9, "mtss_init", "failed: MTSS initialization failed");
                mirage::kprintln!("MTSS [FAILED: {:?}]", error);
            }
        }
        static SPIDER_BOOTRT_IMAGE: mirage::kernel::sync::SpinLock<[u8; 1024 * 1024]> =
            mirage::kernel::sync::SpinLock::new([0; 1024 * 1024]);
        let mut spider_image = SPIDER_BOOTRT_IMAGE.lock();
        let continuation = {
            let mut kso_context = KsoContext::new(
                &boot_info,
                kernel,
                &supervisor,
                boot_runtime.as_ref(),
                &mut spider_image[..],
            );
            kso_context.boot_runtime.deps = boot_deps;
            kso_context.sync_from_deps();
            let continuation = maybe_retry_pid1_handoff_after_mtss_change(&mut kso_context);
            boot_deps = kso_context.boot_runtime.deps;
            continuation
        };
        match continuation {
            BootContinueResult::DispatcherStarted => {
                mirage::kprintln!("SYSTEM DISPATCHER [STARTED]");
            }
            BootContinueResult::DispatcherPending(reason) => {
                mirage::kprintln!(
                    "post-MTSS continuation resolved: dispatcher pending: {}",
                    reason
                );
            }
            BootContinueResult::RootFsUnavailable(reason) => {
                mirage::kprintln!(
                    "post-MTSS continuation resolved: rootfs unavailable: {}",
                    reason
                );
            }
            BootContinueResult::Fatal(reason) => {
                boot_phase_failed(BootPhase::SystemDispatcher, reason);
                mirage::kprintln!("post-MTSS continuation fatal: {}", reason);
            }
        }
        boot_phase_start(BootPhase::BootScreen);
        boot_phase_ok(BootPhase::BootScreen);
        bootflow(21, "scheduler_idleloop", "enter");
        boot_phase_start(BootPhase::IdleLoop);
        boot_phase_running(BootPhase::IdleLoop);
        boot_deps.idleloop_started = true;
        bootflow(21, "scheduler_idleloop", "ok");
        mirage::kprintln!("IDLELOOP [RUNNING]");
        boot_phase_validate_no_unresolved();
        let mut observed_timer_ticks = x86_64::timer_ticks();
        loop {
            if x86_64::poll_debug_shell_hotkey() {
                debug_shell::enter_early_debug_shell(kernel);
            }
            if x86_64::timer_tick_pending(&mut observed_timer_ticks) {
                kernel.tick();
            }
            x86_64::idle_halt();
        }
    }
}
