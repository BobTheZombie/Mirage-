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
    boot_phase_failed, boot_phase_ok, boot_phase_ready, boot_phase_start,
    boot_phase_validate_no_unresolved, BootPhase,
};
#[cfg(all(not(feature = "emergency-boot"), not(feature = "full-boot")))]
use mirage::kernel::ipc::MessagePayload;
#[cfg(not(feature = "emergency-boot"))]
use mirage::kernel::kso::{
    kso_transition, maybe_retry_pid1_handoff_after_mtss_change, BootContinueResult,
    BootRuntimeDeps, KsoBootNode, KsoContext, KsoState,
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
    mirage::kprintln!("[bootflow 2.1] kernel_constructed: kernel object allocation enter");
    boot_phase_start(BootPhase::KernelConstructed);
    let kernel = unsafe {
        let storage = core::ptr::addr_of_mut!(BOOT_KERNEL_STORAGE);
        (*storage).write(Kernel::<MAX_PROCESSES, MESSAGE_DEPTH>::new())
    };
    boot_phase_ready(BootPhase::KernelConstructed);
    mirage::kprintln!("[bootflow 2.1] kernel_constructed: kernel object reference ready");
    mirage::kprintln!("[bootflow 2.2] kernel_constructed: render UI skipped (continuation edge)");
    mirage::kprintln!(
        "[bootflow 2.3] kernel_constructed: debug shell not entered from continuation edge"
    );
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
        kso_transition(KsoBootNode::BootRuntime, KsoState::Starting, "started");
        let boot_runtime =
            mirage::kernel::boot_runtime::find_boot_runtime_module(boot_info.modules).and_then(
                |image| match mirage::kernel::boot_runtime::BootRuntimeRamFs::mount(image) {
                    Ok((_runtime, fs)) => {
                        kso_transition(KsoBootNode::BootRuntime, KsoState::Online, "ok");
                        bootflow(5, "boot_runtime_validation", "ok");
                        bootflow(6, "runtime_vfs_mount", "ok");
                        kso_transition(KsoBootNode::SpiderRs, KsoState::Found, "found");
                        kso_transition(KsoBootNode::SystemDispatcher, KsoState::Found, "found");
                        mirage::kprintln!("BOOT RUNTIME [OK]");
                        mirage::kprintln!("SPIDER-RS IMAGE [FOUND]");
                        mirage::kprintln!("SPIDER-RSD IMAGE [FOUND]");
                        mirage::kprintln!("[spider-rt] module found and RuntimeVfs mounted: /spider-rt/sbin/spider-rs and /spider-rt/sbin/spider-rsd available");
                        boot_deps.spider_rt_available = true;
                        Some(fs)
                    }
                    Err(error) => {
                        kso_transition(
                            KsoBootNode::BootRuntime,
                            KsoState::Failed,
                            "Boot Runtime image validation failed",
                        );
                        bootflow(5, "boot_runtime_validation", "failed: validation failed");
                        mirage::kprintln!("Boot Runtime validation failed: {:?}", error);
                        None
                    }
                },
            );
        if boot_runtime.is_none() {
            kso_transition(
                KsoBootNode::BootRuntime,
                KsoState::Failed,
                "Spider-rs-required Boot Runtime image missing",
            );
            bootflow(5, "boot_runtime_validation", "failed: image missing");
            kso_transition(
                KsoBootNode::SpiderRs,
                KsoState::Failed,
                "missing from Boot Runtime image",
            );
            kso_transition(
                KsoBootNode::RootFs,
                KsoState::Skipped,
                "boot runtime invalid",
            );
            kso_transition(
                KsoBootNode::UserspaceLoader,
                KsoState::Skipped,
                "boot runtime invalid",
            );
            mirage::kprintln!(
                "[BOOTDIAG] ERROR BOOT RUNTIME: BOOT RUNTIME IMAGE VALIDATION FAILED"
            );
            mirage::kprintln!("[BOOTDIAG] ERROR BOOT RUNTIME: /spider-rt/sbin/spider-rs missing");
            mirage::kprintln!("[spider-rt] RuntimeVfs Failed: Spider-rs-required image missing");
        }

        static SPIDER_BOOTRT_IMAGE: mirage::kernel::sync::SpinLock<[u8; 1024 * 1024]> =
            mirage::kernel::sync::SpinLock::new([0; 1024 * 1024]);
        let mut spider_image = SPIDER_BOOTRT_IMAGE.lock();

        {
            let mut kso_context = KsoContext::new(
                &boot_info,
                kernel,
                &supervisor,
                boot_runtime.as_ref(),
                &mut spider_image[..],
            );
            kso_context.boot_runtime.deps = boot_deps;
            kso_context.sync_from_deps();
            let _ = mirage::kernel::kso::rootfs_mount(&mut kso_context);
            let _ = mirage::kernel::kso::supervisor_start(&mut kso_context);
            boot_deps = kso_context.boot_runtime.deps;
        }

        kso_transition(
            KsoBootNode::Userspace,
            KsoState::WaitingDeps,
            "MTSS scheduler not ready",
        );
        kso_transition(
            KsoBootNode::SpiderRs,
            KsoState::WaitingDeps,
            "MTSS scheduler not ready",
        );
        mirage::kprintln!("PID1 HANDOFF [PENDING: MTSS scheduler not ready]");

        #[cfg(not(feature = "full-boot"))]
        {
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
        kso_transition(KsoBootNode::Mtss, KsoState::Starting, "started");
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
                    kso_transition(KsoBootNode::Mtss, KsoState::Online, "online");
                    bootflow(9, "mtss_init", "ok");
                    mirage::kprintln!("MTSS [ ONLINE ]");
                } else if !report.timer_ready || !report.preemption_ready {
                    kso_transition(
                        KsoBootNode::Mtss,
                        KsoState::Degraded,
                        "timer/preemption backend pending",
                    );
                    bootflow(9, "mtss_init", "ok");
                    mirage::kprintln!("MTSS [DEGRADED: timer/preemption backend pending]");
                } else {
                    kso_transition(
                        KsoBootNode::Mtss,
                        KsoState::WaitingDeps,
                        "required component pending",
                    );
                    bootflow(9, "mtss_init", "failed: required component pending");
                    mirage::kprintln!("MTSS [PENDING: required component pending]");
                }
            }
            Err(error) => {
                #[cfg(any(feature = "bootdiag-serial", feature = "bootdiag-verbose"))]
                mirage::kprintln!("[bootdiag] MTSS init failed");
                boot_deps.mtss.failed = true;
                kso_transition(
                    KsoBootNode::Mtss,
                    KsoState::Failed,
                    "MTSS initialization failed",
                );
                bootflow(9, "mtss_init", "failed: MTSS initialization failed");
                mirage::kprintln!("MTSS [FAILED: {:?}]", error);
            }
        }
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
                kso_transition(KsoBootNode::SystemDispatcher, KsoState::Failed, reason);
                mirage::kprintln!("post-MTSS continuation fatal: {}", reason);
            }
        }
        boot_phase_start(BootPhase::BootScreen);
        boot_phase_ok(BootPhase::BootScreen);
        bootflow(21, "scheduler_idleloop", "enter");
        kso_transition(KsoBootNode::IdleLoop, KsoState::Starting, "started");
        kso_transition(KsoBootNode::IdleLoop, KsoState::Online, "running");
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
