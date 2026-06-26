#![no_std]
#![no_main]

extern crate mirage;

#[cfg(not(feature = "emergency-boot"))]
use mirage::arch::x86_64;
use mirage::arch::x86_64::boot::BootInfo;
#[cfg(not(feature = "emergency-boot"))]
use mirage::boot::pid1_retry::{decide_pid1_handoff, Pid1HandoffDecision, RetryReadiness};
#[cfg(not(feature = "emergency-boot"))]
use mirage::kernel::boot_phase::{
    boot_phase_failed, boot_phase_found, boot_phase_ok, boot_phase_online, boot_phase_pending,
    boot_phase_running, boot_phase_skipped, boot_phase_start, boot_phase_stub,
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

#[cfg(not(feature = "emergency-boot"))]
#[derive(Clone, Copy, Debug, Default)]
struct MtssReadiness {
    core_ready: bool,
    scheduler_ready: bool,
    timer_ready: bool,
    preemption_ready: bool,
    idle_ready: bool,
    task_creation_api_ready: bool,
    mark_runnable_api_ready: bool,
    require_preemption_for_userspace: bool,
    failed: bool,
}

#[cfg(not(feature = "emergency-boot"))]
impl MtssReadiness {
    const fn fully_online(&self) -> bool {
        self.core_ready
            && self.scheduler_ready
            && self.timer_ready
            && self.preemption_ready
            && self.idle_ready
            && self.task_creation_api_ready
            && self.mark_runnable_api_ready
            && !self.failed
    }

    const fn pid1_handoff_allowed(&self) -> bool {
        self.pid1_handoff_blocker().is_none()
    }

    const fn pid1_launch_mode(&self) -> Option<mirage::supervisor::pid1::Pid1LaunchMode> {
        if !self.pid1_handoff_allowed() {
            return None;
        }
        if self.preemption_ready {
            Some(mirage::supervisor::pid1::Pid1LaunchMode::Preemptive)
        } else {
            Some(mirage::supervisor::pid1::Pid1LaunchMode::Cooperative)
        }
    }

    const fn pid1_handoff_blocker(&self) -> Option<&'static str> {
        if self.failed {
            return Some("MTSS failed");
        }
        if !self.core_ready {
            return Some("MTSS core not ready");
        }
        if !self.scheduler_ready {
            return Some("MTSS scheduler not ready");
        }
        if !self.idle_ready {
            return Some("idle task unavailable");
        }
        if !self.task_creation_api_ready {
            return Some("task creation API unavailable");
        }
        if !self.mark_runnable_api_ready {
            return Some("mark_runnable unavailable");
        }
        if self.require_preemption_for_userspace && !self.preemption_ready {
            return Some("policy requires preemption before userspace");
        }
        None
    }
}

#[cfg(not(feature = "emergency-boot"))]
#[derive(Clone, Copy, Debug, Default)]
struct BootRuntimeDeps {
    root_fs_resolved: bool,
    root_fs_online: bool,
    supervisor_online: bool,
    mtss: MtssReadiness,
    spider_rt_available: bool,
    spider_found: bool,
    spider_elf_ok: bool,
    userspace_loader_started: bool,
    userspace_launch_deferred: bool,
    pid1_created: bool,
    pid1_runnable: bool,
    dispatcher_started: bool,
    dispatcher_pending: bool,
    idleloop_started: bool,
}

#[cfg(not(feature = "emergency-boot"))]
impl BootRuntimeDeps {
    const fn pid1_handoff_allowed(&self) -> bool {
        self.mtss.pid1_handoff_allowed()
    }

    const fn pid1_handoff_blocker(&self) -> &'static str {
        match self.mtss.pid1_handoff_blocker() {
            Some(blocker) => blocker,
            None => "PID1 handoff allowed",
        }
    }
}

#[cfg(not(feature = "emergency-boot"))]
fn bootflow(seq: u8, phase: &'static str, status: &'static str) {
    mirage::kprintln!("[bootflow {}] phase={} {}", seq, phase, status);
}

#[cfg(not(feature = "emergency-boot"))]
fn pid1_retry_readiness(deps: &BootRuntimeDeps) -> RetryReadiness {
    RetryReadiness {
        rootfs_online: deps.root_fs_online,
        supervisor_online: deps.supervisor_online,
        boot_runtime_available: deps.spider_rt_available,
        spider_rs_available: deps.spider_rt_available,
        mtss_core_ready: deps.mtss.core_ready,
        mtss_scheduler_ready: deps.mtss.scheduler_ready,
        mtss_idle_ready: deps.mtss.idle_ready,
        mtss_task_api_ready: deps.mtss.task_creation_api_ready,
        mtss_mark_runnable_ready: deps.mtss.mark_runnable_api_ready,
        mtss_preemption_ready: deps.mtss.preemption_ready,
        require_preemption_for_userspace: deps.mtss.require_preemption_for_userspace,
        mtss_failed: deps.mtss.failed,
    }
}

#[cfg(not(feature = "emergency-boot"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pid1LaunchState {
    Deferred(&'static str),
    Runnable,
}

#[cfg(not(feature = "emergency-boot"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
enum BootContinueResult {
    DispatcherStarted,
    DispatcherPending(&'static str),
    RootFsUnavailable(&'static str),
    Fatal(&'static str),
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
    bootflow(17, "pid1_handoff_eligibility", "enter");
    if !deps.root_fs_online {
        deps.userspace_launch_deferred = true;
        bootflow(17, "pid1_handoff_eligibility", "failed: root FS not online");
        mirage::kprintln!("SPIDER-RS PID1 [PENDING: root FS not online]");
        return Ok(Pid1LaunchState::Deferred("root FS not online"));
    }
    if !deps.supervisor_online {
        deps.userspace_launch_deferred = true;
        bootflow(
            17,
            "pid1_handoff_eligibility",
            "failed: supervisor not online",
        );
        mirage::kprintln!("SPIDER-RS PID1 [PENDING: supervisor not online]");
        return Ok(Pid1LaunchState::Deferred("supervisor not online"));
    }
    if !deps.pid1_handoff_allowed() {
        let blocker = deps.pid1_handoff_blocker();
        deps.userspace_launch_deferred = true;
        bootflow(
            17,
            "pid1_handoff_eligibility",
            "failed: MTSS handoff blocked",
        );
        mirage::kprintln!("SPIDER-RS PID1 [PENDING: {}]", blocker);
        return Ok(Pid1LaunchState::Deferred(blocker));
    }
    if !deps.spider_rt_available {
        deps.userspace_launch_deferred = true;
        bootflow(
            17,
            "pid1_handoff_eligibility",
            "failed: spider runtime unavailable",
        );
        mirage::kprintln!("SPIDER-RS PID1 [PENDING: spider runtime unavailable]");
        return Ok(Pid1LaunchState::Deferred("spider runtime unavailable"));
    }
    bootflow(17, "pid1_handoff_eligibility", "ok");

    bootflow(18, "userspace_loader", "enter");
    boot_phase_start(BootPhase::UserspaceLoader);
    deps.userspace_loader_started = true;
    mirage::kprintln!("USERSPACE LOADER [STARTED]");
    let fs = match boot_runtime {
        Some(fs) => fs,
        None => {
            deps.userspace_launch_deferred = true;
            bootflow(18, "userspace_loader", "failed: spider runtime unavailable");
            mirage::kprintln!("SPIDER-RS PID1 [PENDING: spider runtime unavailable]");
            return Ok(Pid1LaunchState::Deferred("spider runtime unavailable"));
        }
    };
    bootflow(19, "spider_rs_elf_load", "enter");
    let len = match fs.read(
        mirage::kernel::boot_runtime::BOOTRT_MOUNTED_ENTRY,
        0,
        spider_image,
    ) {
        Ok(len) => len,
        Err(_) => {
            bootflow(19, "spider_rs_elf_load", "failed: Spider-rs binary missing");
            mirage::kprintln!("SPIDER-RS PID1 [FAILED: Spider-rs binary missing]");
            return Err(mirage::supervisor::pid1::SpiderPid1LaunchError::Handoff(
                mirage::supervisor::pid1::SpiderPid1HandoffError::SpiderBinaryMissing,
            ));
        }
    };
    bootflow(19, "spider_rs_elf_load", "ok");
    boot_phase_ok(BootPhase::UserspaceLoader);
    bootflow(18, "userspace_loader", "ok");
    boot_phase_start(BootPhase::SpiderRs);
    boot_phase_ok(BootPhase::SpiderRs);
    deps.spider_found = true;
    mirage::kprintln!("SPIDER-RS IMAGE [FOUND]");

    bootflow(20, "spider_rs_pid1_task_creation", "enter");
    let report = supervisor.launch_spider_rs_pid1_checked(
        kernel,
        &spider_image[..len],
        mirage::supervisor::pid1::SpiderPid1Preconditions {
            root_fs_online: deps.root_fs_online,
            runtime_vfs_mounted: boot_runtime.is_some(),
            spider_binary_present: len > 0,
            mtss_pid1_handoff_allowed: deps.pid1_handoff_allowed(),
            mtss_launch_mode: deps.mtss.pid1_launch_mode(),
            mtss_blocker: deps.mtss.pid1_handoff_blocker(),
            userspace_loader_ready: deps.userspace_loader_started,
        },
    );
    deps.userspace_launch_deferred = report.blocker().is_some();
    deps.spider_elf_ok = report.entry_preflight_ok;
    deps.pid1_created = report.process_created;
    deps.pid1_runnable = report.accepted_into_run_queue;
    if let Some(blocker) = report.blocker() {
        bootflow(
            20,
            "spider_rs_pid1_task_creation",
            "failed: launch report blocked",
        );
        if report.process_created || report.main_thread_created || report.entry_preflight_ok {
            mirage::kprintln!("SPIDER-RS PID1 [PENDING: {}]", blocker);
            return Ok(Pid1LaunchState::Deferred(blocker));
        }
        mirage::kprintln!("SPIDER-RS PID1 [FAILED: {}]", blocker);
        return Ok(Pid1LaunchState::Deferred(blocker));
    }
    bootflow(20, "spider_rs_pid1_task_creation", "ok");
    boot_phase_ok(BootPhase::SpiderRs);
    boot_phase_stub(
        BootPhase::Pid1,
        "PENDING: ring3 transition not implemented after MTSS admission",
    );
    boot_phase_start(BootPhase::SystemDispatcher);
    boot_phase_stub(
        BootPhase::SystemDispatcher,
        "PENDING: user-mode transition not implemented",
    );
    boot_phase_stub(BootPhase::M1Terminal, "PENDING: dispatcher not online");
    boot_phase_stub(
        BootPhase::Userspace,
        "PID1 runnable; user-mode transition pending",
    );
    mirage::kprintln!("SPIDER-RS ELF [OK]");
    mirage::kprintln!("SPIDER-RS PID1 [CREATED]");
    mirage::kprintln!("SPIDER-RS PID1 [RUNNABLE]");
    mirage::kprintln!("SPIDER-RS PID1 [ PENDING: ring3 transition not implemented ]");
    deps.dispatcher_started = false;
    deps.dispatcher_pending = true;
    mirage::kprintln!("SPIDER-RSD [PENDING: user-mode transition not implemented]");
    mirage::kprintln!("SYSTEM DISPATCHER [PENDING: user-mode transition not implemented]");
    mirage::kprintln!("M1 TERMINAL [PENDING: dispatcher not online]");
    mirage::kprintln!(
        "[pid1] process created pid={:?} entry={:#x} bytes={} path={}",
        report.pid,
        report.entry.map(|entry| entry.0).unwrap_or(0),
        report.image_len,
        report.runtime_path
    );
    // Ring-3 dispatch is still pending. Do not print the terminal payload here;
    // m1-terminal must be launched by spider-rsd once dispatcher child
    // launch and the console ABI exist.
    mirage::kprintln!("Userspace [PENDING: user-mode transition not implemented]");
    Ok(Pid1LaunchState::Runnable)
}

#[cfg(not(feature = "emergency-boot"))]
fn maybe_retry_pid1_handoff_after_mtss_change<const NPROC: usize, const MSG_DEPTH: usize>(
    deps: &mut BootRuntimeDeps,
    supervisor: &Supervisor,
    kernel: &mut Kernel<NPROC, MSG_DEPTH>,
    boot_runtime: Option<&mirage::kernel::boot_runtime::BootRuntimeRamFs>,
    spider_image: &mut [u8],
) -> BootContinueResult {
    if deps.mtss.failed {
        return BootContinueResult::Fatal("MTSS failed");
    }

    deps.root_fs_resolved = true;
    let handoff_decision = decide_pid1_handoff(pid1_retry_readiness(deps));
    match handoff_decision {
        Pid1HandoffDecision::AllowedCooperative | Pid1HandoffDecision::AllowedPreemptive => {
            deps.userspace_launch_deferred = false;
            mirage::kprintln!("{}", handoff_decision.status_message());
        }
        Pid1HandoffDecision::Pending(reason) => {
            deps.userspace_launch_deferred = true;
            mirage::kprintln!("{}", handoff_decision.status_message());
            if reason != "root FS not online" {
                boot_phase_skipped(BootPhase::UserspaceLoader, reason);
                boot_phase_stub(BootPhase::Userspace, reason);
                boot_phase_stub(BootPhase::SystemDispatcher, reason);
                mirage::kprintln!("USERSPACE LOADER [SKIPPED: {}]", reason);
                mirage::kprintln!("SYSTEM DISPATCHER [PENDING: {}]", reason);
                return BootContinueResult::DispatcherPending(reason);
            }
        }
        Pid1HandoffDecision::Fatal(reason) => return BootContinueResult::Fatal(reason),
    }

    if !deps.root_fs_online {
        boot_phase_skipped(BootPhase::UserspaceLoader, "rootfs unavailable");
        boot_phase_skipped(BootPhase::SpiderRs, "rootfs unavailable");
        boot_phase_skipped(BootPhase::Pid1, "rootfs unavailable");
        boot_phase_stub(BootPhase::SystemDispatcher, "PENDING: rootfs unavailable");
        boot_phase_stub(BootPhase::Userspace, "SKIPPED: rootfs unavailable");
        mirage::kprintln!("USERSPACE LOADER [SKIPPED: rootfs unavailable]");
        mirage::kprintln!("SPIDER-RS IMAGE [SKIPPED: rootfs unavailable]");
        mirage::kprintln!("SYSTEM DISPATCHER [PENDING: rootfs unavailable]");
        return BootContinueResult::RootFsUnavailable("rootfs unavailable");
    }

    match maybe_launch_pid1(deps, supervisor, kernel, boot_runtime, spider_image) {
        Ok(Pid1LaunchState::Runnable) => {
            BootContinueResult::DispatcherPending("user-mode transition not implemented")
        }
        Ok(Pid1LaunchState::Deferred(reason)) => {
            boot_phase_skipped(BootPhase::UserspaceLoader, reason);
            boot_phase_stub(BootPhase::Userspace, reason);
            boot_phase_stub(BootPhase::SystemDispatcher, reason);
            mirage::kprintln!("USERSPACE LOADER [SKIPPED: {}]", reason);
            mirage::kprintln!("SYSTEM DISPATCHER [PENDING: {}]", reason);
            BootContinueResult::DispatcherPending(reason)
        }
        Err(error) => {
            boot_phase_failed(BootPhase::Userspace, "PID1 launch failed");
            boot_phase_stub(
                BootPhase::SystemDispatcher,
                "PENDING: Spider-rs PID1 launch failed",
            );
            mirage::kprintln!("Spider-rs PID 1 not launched: {:?}", error);
            mirage::kprintln!("SYSTEM DISPATCHER [PENDING: Spider-rs PID1 launch failed]");
            BootContinueResult::DispatcherPending("Spider-rs PID1 launch failed")
        }
    }
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
        bootflow(2, "kernel_constructed", "enter");
        boot_phase_start(BootPhase::KernelConstructed);
        let mut kernel = Kernel::<MAX_PROCESSES, MESSAGE_DEPTH>::new();
        boot_phase_ok(BootPhase::KernelConstructed);
        bootflow(2, "kernel_constructed", "ok");
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
            let service_report = supervisor.bootstrap_services(&mut kernel);
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
            let minimal_report = supervisor.bootstrap_minimal(&mut kernel);
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
        let continuation = maybe_retry_pid1_handoff_after_mtss_change(
            &mut boot_deps,
            &supervisor,
            &mut kernel,
            boot_runtime.as_ref(),
            &mut spider_image[..],
        );
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
                debug_shell::enter_early_debug_shell(&mut kernel);
            }
            if x86_64::timer_tick_pending(&mut observed_timer_ticks) {
                kernel.tick();
            }
            x86_64::idle_halt();
        }
    }
}
