//! KSO state and stable identifiers.

/// Stable identifier for a kernel service object node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KsoNodeId(pub u16);

/// Runtime lifecycle state tracked by the deterministic KSO runner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KsoState {
    New,
    WaitingDeps,
    Starting,
    Online,
    Degraded,
    Skipped,
    Disabled,
    Failed,
}

/// Public status snapshot for a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KsoStatus {
    pub id: KsoNodeId,
    pub state: KsoState,
    pub blocker: Option<&'static str>,
}

impl KsoStatus {
    /// Writes a stable human-readable blocker such as `waiting for: mtss.scheduler`.
    pub fn blocker_message<'a>(&self, buffer: &'a mut [u8]) -> Option<&'a str> {
        let blocker = self.blocker?;
        let prefix = b"waiting for: ";
        let bytes = blocker.as_bytes();
        if buffer.len() < prefix.len() + bytes.len() {
            return None;
        }
        buffer[..prefix.len()].copy_from_slice(prefix);
        buffer[prefix.len()..prefix.len() + bytes.len()].copy_from_slice(bytes);
        core::str::from_utf8(&buffer[..prefix.len() + bytes.len()]).ok()
    }
}

/// Result returned by a node startup thunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KsoStartResult {
    Started,
    StartedDegraded,
    Skipped,
    Disabled,
    Failed,
}

/// Deterministic run result for a runner pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KsoRunOutcome {
    Complete,
    Progress,
    Blocked,
    Fatal {
        node: KsoNodeId,
        reason: &'static str,
    },
}

use crate::arch::x86_64::boot::BootInfo;
use crate::boot::pid1_retry::{decide_pid1_handoff, Pid1HandoffDecision, RetryReadiness};
use crate::kernel::boot_phase::{
    boot_phase_failed, boot_phase_ok, boot_phase_skipped, boot_phase_start, boot_phase_stub,
    BootPhase,
};
use crate::kernel::boot_runtime::BootRuntimeRamFs;
use crate::kernel::Kernel;
use crate::supervisor::pid1::{Pid1LaunchMode, SpiderPid1LaunchError};
use crate::supervisor::Supervisor;

#[derive(Clone, Copy, Debug, Default)]
pub struct MtssReadiness {
    pub core_ready: bool,
    pub scheduler_ready: bool,
    pub timer_ready: bool,
    pub preemption_ready: bool,
    pub idle_ready: bool,
    pub task_creation_api_ready: bool,
    pub mark_runnable_api_ready: bool,
    pub require_preemption_for_userspace: bool,
    pub failed: bool,
}

impl MtssReadiness {
    pub const fn fully_online(&self) -> bool {
        self.core_ready
            && self.scheduler_ready
            && self.timer_ready
            && self.preemption_ready
            && self.idle_ready
            && self.task_creation_api_ready
            && self.mark_runnable_api_ready
            && !self.failed
    }

    pub const fn pid1_handoff_allowed(&self) -> bool {
        self.pid1_handoff_blocker().is_none()
    }

    pub const fn pid1_launch_mode(&self) -> Option<Pid1LaunchMode> {
        if !self.pid1_handoff_allowed() {
            return None;
        }
        if self.preemption_ready {
            Some(Pid1LaunchMode::Preemptive)
        } else {
            Some(Pid1LaunchMode::Cooperative)
        }
    }

    pub const fn pid1_handoff_blocker(&self) -> Option<&'static str> {
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

#[derive(Clone, Copy, Debug, Default)]
pub struct BootRuntimeDeps {
    pub root_fs_resolved: bool,
    pub root_fs_online: bool,
    pub supervisor_online: bool,
    pub mtss: MtssReadiness,
    pub spider_rt_available: bool,
    pub spider_found: bool,
    pub spider_elf_ok: bool,
    pub userspace_loader_started: bool,
    pub userspace_launch_deferred: bool,
    pub pid1_created: bool,
    pub pid1_runnable: bool,
    pub dispatcher_started: bool,
    pub dispatcher_pending: bool,
    pub idleloop_started: bool,
}

impl BootRuntimeDeps {
    pub const fn pid1_handoff_allowed(&self) -> bool {
        self.mtss.pid1_handoff_allowed()
    }

    pub const fn pid1_handoff_blocker(&self) -> &'static str {
        match self.mtss.pid1_handoff_blocker() {
            Some(blocker) => blocker,
            None => "PID1 handoff allowed",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BootRuntimeState<'boot> {
    pub deps: BootRuntimeDeps,
    pub ramfs: Option<&'boot BootRuntimeRamFs>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RootFsState {
    pub resolved: bool,
    pub online: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UserspaceLoaderState {
    pub started: bool,
    pub launch_deferred: bool,
}

pub struct KsoContext<'boot, 'kernel, const NPROC: usize, const MSG_DEPTH: usize> {
    pub boot_info: &'boot BootInfo,
    pub kernel: &'kernel mut Kernel<NPROC, MSG_DEPTH>,
    pub supervisor: &'boot Supervisor,
    pub boot_runtime: BootRuntimeState<'boot>,
    pub mtss: MtssReadiness,
    pub rootfs: RootFsState,
    pub userspace_loader: UserspaceLoaderState,
    pub spider_image: &'kernel mut [u8],
}

impl<'boot, 'kernel, const NPROC: usize, const MSG_DEPTH: usize>
    KsoContext<'boot, 'kernel, NPROC, MSG_DEPTH>
{
    pub fn new(
        boot_info: &'boot BootInfo,
        kernel: &'kernel mut Kernel<NPROC, MSG_DEPTH>,
        supervisor: &'boot Supervisor,
        boot_runtime: Option<&'boot BootRuntimeRamFs>,
        spider_image: &'kernel mut [u8],
    ) -> Self {
        Self {
            boot_info,
            kernel,
            supervisor,
            boot_runtime: BootRuntimeState {
                deps: BootRuntimeDeps::default(),
                ramfs: boot_runtime,
            },
            mtss: MtssReadiness::default(),
            rootfs: RootFsState::default(),
            userspace_loader: UserspaceLoaderState::default(),
            spider_image,
        }
    }

    pub fn sync_from_deps(&mut self) {
        self.mtss = self.boot_runtime.deps.mtss;
        self.rootfs = RootFsState {
            resolved: self.boot_runtime.deps.root_fs_resolved,
            online: self.boot_runtime.deps.root_fs_online,
        };
        self.userspace_loader = UserspaceLoaderState {
            started: self.boot_runtime.deps.userspace_loader_started,
            launch_deferred: self.boot_runtime.deps.userspace_launch_deferred,
        };
    }
}

pub fn pid1_retry_readiness(deps: &BootRuntimeDeps) -> RetryReadiness {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pid1LaunchState {
    Deferred(&'static str),
    Runnable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub enum BootContinueResult {
    DispatcherStarted,
    DispatcherPending(&'static str),
    RootFsUnavailable(&'static str),
    Fatal(&'static str),
}

fn bootflow(seq: u8, phase: &'static str, status: &'static str) {
    crate::kprintln!("[bootflow {}] phase={} {}", seq, phase, status);
}

pub fn maybe_launch_pid1<const NPROC: usize, const MSG_DEPTH: usize>(
    ctx: &mut KsoContext<'_, '_, NPROC, MSG_DEPTH>,
) -> Result<Pid1LaunchState, SpiderPid1LaunchError> {
    let deps = &mut ctx.boot_runtime.deps;
    if deps.pid1_runnable {
        return Ok(Pid1LaunchState::Runnable);
    }
    bootflow(17, "pid1_handoff_eligibility", "enter");
    if !deps.root_fs_online {
        deps.userspace_launch_deferred = true;
        bootflow(17, "pid1_handoff_eligibility", "failed: root FS not online");
        crate::kprintln!("SPIDER-RS PID1 [PENDING: root FS not online]");
        return Ok(Pid1LaunchState::Deferred("root FS not online"));
    }
    if !deps.supervisor_online {
        deps.userspace_launch_deferred = true;
        bootflow(
            17,
            "pid1_handoff_eligibility",
            "failed: supervisor not online",
        );
        crate::kprintln!("SPIDER-RS PID1 [PENDING: supervisor not online]");
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
        crate::kprintln!("SPIDER-RS PID1 [PENDING: {}]", blocker);
        return Ok(Pid1LaunchState::Deferred(blocker));
    }
    if !deps.spider_rt_available {
        deps.userspace_launch_deferred = true;
        bootflow(
            17,
            "pid1_handoff_eligibility",
            "failed: spider runtime unavailable",
        );
        crate::kprintln!("SPIDER-RS PID1 [PENDING: spider runtime unavailable]");
        return Ok(Pid1LaunchState::Deferred("spider runtime unavailable"));
    }
    bootflow(17, "pid1_handoff_eligibility", "ok");

    bootflow(18, "userspace_loader", "enter");
    boot_phase_start(BootPhase::UserspaceLoader);
    deps.userspace_loader_started = true;
    ctx.userspace_loader.started = true;
    crate::kprintln!("USERSPACE LOADER [STARTED]");
    let fs = match ctx.boot_runtime.ramfs {
        Some(fs) => fs,
        None => {
            deps.userspace_launch_deferred = true;
            ctx.userspace_loader.launch_deferred = true;
            bootflow(18, "userspace_loader", "failed: spider runtime unavailable");
            crate::kprintln!("SPIDER-RS PID1 [PENDING: spider runtime unavailable]");
            return Ok(Pid1LaunchState::Deferred("spider runtime unavailable"));
        }
    };
    bootflow(19, "spider_rs_elf_load", "enter");
    let len = match fs.read(
        crate::kernel::boot_runtime::BOOTRT_MOUNTED_ENTRY,
        0,
        ctx.spider_image,
    ) {
        Ok(len) => len,
        Err(_) => {
            bootflow(19, "spider_rs_elf_load", "failed: Spider-rs binary missing");
            crate::kprintln!("SPIDER-RS PID1 [FAILED: Spider-rs binary missing]");
            return Err(crate::supervisor::pid1::SpiderPid1LaunchError::Handoff(
                crate::supervisor::pid1::SpiderPid1HandoffError::SpiderBinaryMissing,
            ));
        }
    };
    bootflow(19, "spider_rs_elf_load", "ok");
    boot_phase_ok(BootPhase::UserspaceLoader);
    bootflow(18, "userspace_loader", "ok");
    boot_phase_start(BootPhase::SpiderRs);
    boot_phase_ok(BootPhase::SpiderRs);
    deps.spider_found = true;
    crate::kprintln!("SPIDER-RS IMAGE [FOUND]");

    bootflow(20, "spider_rs_pid1_task_creation", "enter");
    let report = ctx.supervisor.launch_spider_rs_pid1_checked(
        ctx.kernel,
        &ctx.spider_image[..len],
        crate::supervisor::pid1::SpiderPid1Preconditions {
            root_fs_online: deps.root_fs_online,
            runtime_vfs_mounted: ctx.boot_runtime.ramfs.is_some(),
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
            crate::kprintln!("SPIDER-RS PID1 [PENDING: {}]", blocker);
            return Ok(Pid1LaunchState::Deferred(blocker));
        }
        crate::kprintln!("SPIDER-RS PID1 [FAILED: {}]", blocker);
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
    crate::kprintln!("SPIDER-RS ELF [OK]");
    crate::kprintln!("SPIDER-RS PID1 [CREATED]");
    crate::kprintln!("SPIDER-RS PID1 [RUNNABLE]");
    crate::kprintln!("SPIDER-RS PID1 [ PENDING: ring3 transition not implemented ]");
    deps.dispatcher_started = false;
    deps.dispatcher_pending = true;
    crate::kprintln!("SPIDER-RSD [PENDING: user-mode transition not implemented]");
    crate::kprintln!("SYSTEM DISPATCHER [PENDING: user-mode transition not implemented]");
    crate::kprintln!("M1 TERMINAL [PENDING: dispatcher not online]");
    crate::kprintln!(
        "[pid1] process created pid={:?} entry={:#x} bytes={} path={}",
        report.pid,
        report.entry.map(|entry| entry.0).unwrap_or(0),
        report.image_len,
        report.runtime_path
    );
    crate::kprintln!("Userspace [PENDING: user-mode transition not implemented]");
    Ok(Pid1LaunchState::Runnable)
}

pub fn maybe_retry_pid1_handoff_after_mtss_change<const NPROC: usize, const MSG_DEPTH: usize>(
    ctx: &mut KsoContext<'_, '_, NPROC, MSG_DEPTH>,
) -> BootContinueResult {
    let deps = &mut ctx.boot_runtime.deps;
    if deps.mtss.failed {
        return BootContinueResult::Fatal("MTSS failed");
    }

    deps.root_fs_resolved = true;
    ctx.rootfs.resolved = true;
    let handoff_decision = decide_pid1_handoff(pid1_retry_readiness(deps));
    match handoff_decision {
        Pid1HandoffDecision::AllowedCooperative | Pid1HandoffDecision::AllowedPreemptive => {
            deps.userspace_launch_deferred = false;
            ctx.userspace_loader.launch_deferred = false;
            crate::kprintln!("{}", handoff_decision.status_message());
        }
        Pid1HandoffDecision::Pending(reason) => {
            deps.userspace_launch_deferred = true;
            ctx.userspace_loader.launch_deferred = true;
            crate::kprintln!("{}", handoff_decision.status_message());
            if reason != "root FS not online" {
                boot_phase_skipped(BootPhase::UserspaceLoader, reason);
                boot_phase_stub(BootPhase::Userspace, reason);
                boot_phase_stub(BootPhase::SystemDispatcher, reason);
                crate::kprintln!("USERSPACE LOADER [SKIPPED: {}]", reason);
                crate::kprintln!("SYSTEM DISPATCHER [PENDING: {}]", reason);
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
        crate::kprintln!("USERSPACE LOADER [SKIPPED: rootfs unavailable]");
        crate::kprintln!("SPIDER-RS IMAGE [SKIPPED: rootfs unavailable]");
        crate::kprintln!("SYSTEM DISPATCHER [PENDING: rootfs unavailable]");
        return BootContinueResult::RootFsUnavailable("rootfs unavailable");
    }

    match maybe_launch_pid1(ctx) {
        Ok(Pid1LaunchState::Runnable) => {
            BootContinueResult::DispatcherPending("user-mode transition not implemented")
        }
        Ok(Pid1LaunchState::Deferred(reason)) => {
            boot_phase_skipped(BootPhase::UserspaceLoader, reason);
            boot_phase_stub(BootPhase::Userspace, reason);
            boot_phase_stub(BootPhase::SystemDispatcher, reason);
            crate::kprintln!("USERSPACE LOADER [SKIPPED: {}]", reason);
            crate::kprintln!("SYSTEM DISPATCHER [PENDING: {}]", reason);
            BootContinueResult::DispatcherPending(reason)
        }
        Err(error) => {
            boot_phase_failed(BootPhase::Userspace, "PID1 launch failed");
            boot_phase_stub(
                BootPhase::SystemDispatcher,
                "PENDING: Spider-rs PID1 launch failed",
            );
            crate::kprintln!("Spider-rs PID 1 not launched: {:?}", error);
            crate::kprintln!("SYSTEM DISPATCHER [PENDING: Spider-rs PID1 launch failed]");
            BootContinueResult::DispatcherPending("Spider-rs PID1 launch failed")
        }
    }
}
