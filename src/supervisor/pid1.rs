//! Supervisor-owned Spider-rs PID 1 launch policy.
//!
//! This module keeps the PID 1 authority decision in the Supervisor while routing
//! actual task admission through the kernel/MTSS boundary.  The kernel still owns
//! ELF validation/mapping mechanics and MTSS owns runnable state; the Supervisor
//! only decides whether this runtime image may become Spider-rs PID 1.

use crate::kernel::process::ProcessId;
use crate::kernel::userspace::{validate_elf64, LoadError, VirtAddr};
use crate::kernel::{Kernel, KernelError};
use crate::supervisor::Supervisor;

/// Stable path Spider-rs is expected to occupy inside RuntimeVfs.
pub const SPIDER_RS_RUNTIME_PATH: &str = "/spider-rt/sbin/spider-rs";

/// First child app declared for Spider-rs once dispatcher child launching exists.
pub const M1_TERMINAL_PATH: &str = "/usr/bin/m1-terminal";
pub const M1_TERMINAL_OUTPUT: &str = "Mirage M1.1 System\nhello world\n";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SpiderPid1Preconditions {
    pub root_fs_online: bool,
    pub runtime_vfs_mounted: bool,
    pub spider_binary_present: bool,
    pub mtss_online: bool,
    pub userspace_loader_ready: bool,
}

/// Supervisor -> MTSS PID 1 handoff record.
///
/// This report is intentionally returned for both complete and incomplete
/// launch attempts so boot code can mark PID 1 milestones only after the real
/// stage has completed, and can print the exact blocker otherwise.
#[derive(Clone, Copy, Debug)]
pub struct SpiderPid1LaunchReport {
    pub pid: Option<ProcessId>,
    pub process_created: bool,
    pub main_thread_created: bool,
    pub entry_preflight_ok: bool,
    pub stack_preflight_ok: bool,
    pub mtss_task_id: Option<mirage_mtss::CoreTaskId>,
    pub mtss_thread_id: Option<mirage_mtss::CoreThreadId>,
    pub accepted_into_run_queue: bool,
    pub entry: Option<VirtAddr>,
    pub image_len: usize,
    pub runtime_path: &'static str,
    pub authorized_by_supervisor: bool,
    pub admitted_through_mtss: bool,
    pub handoff_state: SpiderPid1HandoffState,
    pub dispatcher_state: SpiderDispatcherState,
    pub terminal_manifest: TerminalManifestEntry,
    pub launch_blocker: Option<&'static str>,
}

impl SpiderPid1LaunchReport {
    pub const fn blocker(&self) -> Option<&'static str> {
        self.launch_blocker
    }

    pub const fn is_runnable(&self) -> bool {
        self.process_created
            && self.main_thread_created
            && self.entry_preflight_ok
            && self.stack_preflight_ok
            && self.accepted_into_run_queue
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalManifestEntry {
    pub path: &'static str,
    pub expected_output: &'static str,
    pub state: TerminalLaunchState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalLaunchState {
    PendingDispatcherChildLaunch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpiderDispatcherState {
    Started,
    Pending(&'static str),
    Online,
}

/// Honest milestones in the Spider-rs PID 1 handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpiderPid1HandoffState {
    NotStarted,
    RuntimeUnavailable,
    RuntimeFound,
    ElfValidated,
    ProcessCreated,
    MtssTaskCreated,
    Runnable,
    DispatcherStarted,
    DispatcherPending(&'static str),
    Failed(SpiderPid1HandoffError),
}

/// Failure points in the Spider-rs PID 1 handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpiderPid1HandoffError {
    RuntimeVfsUnavailable,
    SpiderBinaryMissing,
    InvalidElf,
    UnsupportedElfArch,
    SegmentMapFailed,
    StackAllocationFailed,
    ProcessCreateFailed,
    SupervisorDenied,
    MtssUnavailable,
    MtssSpawnFailed,
    DispatcherUnavailable,
    UserModeTransitionUnavailable,
}

#[derive(Debug)]
pub enum SpiderPid1LaunchError {
    Handoff(SpiderPid1HandoffError),
    Load(LoadError),
    Kernel(KernelError),
}

impl Supervisor {
    /// Authorize Spider-rs and hand the runtime image to the MTSS-backed kernel
    /// PID 1 admission path.
    ///
    /// This is the intended Mirage bring-up chain:
    ///
    /// ```text
    /// RuntimeVfs /spider-rt/sbin/spider-rs
    ///     -> Supervisor policy authorization
    ///     -> kernel userspace ELF validation
    ///     -> MTSS PID 1 admission
    ///     -> scheduler/timer entry path
    /// ```
    pub fn launch_spider_rs_pid1_via_mtss<const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        image: &[u8],
    ) -> Result<SpiderPid1LaunchReport, SpiderPid1LaunchError> {
        let report = self.launch_spider_rs_pid1_checked(
            kernel,
            image,
            SpiderPid1Preconditions {
                root_fs_online: true,
                runtime_vfs_mounted: true,
                spider_binary_present: !image.is_empty(),
                mtss_online: true,
                userspace_loader_ready: true,
            },
        );
        match report.blocker() {
            Some(_) => Err(SpiderPid1LaunchError::Handoff(
                SpiderPid1HandoffError::MtssSpawnFailed,
            )),
            None => Ok(report),
        }
    }

    pub fn launch_spider_rs_pid1_checked<const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        image: &[u8],
        preconditions: SpiderPid1Preconditions,
    ) -> SpiderPid1LaunchReport {
        let _ = self;
        let mut report = SpiderPid1LaunchReport {
            pid: None,
            process_created: false,
            main_thread_created: false,
            entry_preflight_ok: false,
            stack_preflight_ok: false,
            mtss_task_id: None,
            mtss_thread_id: None,
            accepted_into_run_queue: false,
            entry: None,
            image_len: image.len(),
            runtime_path: SPIDER_RS_RUNTIME_PATH,
            authorized_by_supervisor: false,
            admitted_through_mtss: false,
            handoff_state: SpiderPid1HandoffState::NotStarted,
            dispatcher_state: SpiderDispatcherState::Pending("Spider-rs PID1 launch incomplete"),
            terminal_manifest: TerminalManifestEntry {
                path: M1_TERMINAL_PATH,
                expected_output: M1_TERMINAL_OUTPUT,
                state: TerminalLaunchState::PendingDispatcherChildLaunch,
            },
            launch_blocker: None,
        };

        if !preconditions.root_fs_online {
            report.handoff_state = SpiderPid1HandoffState::RuntimeUnavailable;
            report.launch_blocker = Some("root FS not online");
            return report;
        }
        if !preconditions.runtime_vfs_mounted {
            report.handoff_state = SpiderPid1HandoffState::RuntimeUnavailable;
            report.launch_blocker = Some("runtime VFS not mounted");
            return report;
        }
        if !preconditions.spider_binary_present || image.is_empty() {
            report.handoff_state =
                SpiderPid1HandoffState::Failed(SpiderPid1HandoffError::SpiderBinaryMissing);
            report.launch_blocker = Some("Spider-rs binary missing");
            return report;
        }
        if !preconditions.mtss_online {
            report.launch_blocker = Some("MTSS not online");
            return report;
        }
        if !preconditions.userspace_loader_ready {
            report.launch_blocker = Some("userspace loader not ready");
            return report;
        }
        report.authorized_by_supervisor = true;
        report.handoff_state = SpiderPid1HandoffState::RuntimeFound;

        match validate_elf64(image) {
            Ok(entry) => {
                report.entry = Some(entry);
                report.entry_preflight_ok = true;
                report.handoff_state = SpiderPid1HandoffState::ElfValidated;
            }
            Err(LoadError::UnsupportedMachine) => {
                report.handoff_state =
                    SpiderPid1HandoffState::Failed(SpiderPid1HandoffError::UnsupportedElfArch);
                report.launch_blocker = Some("unsupported Spider-rs ELF architecture");
                return report;
            }
            Err(_) => {
                report.handoff_state =
                    SpiderPid1HandoffState::Failed(SpiderPid1HandoffError::InvalidElf);
                report.launch_blocker = Some("Spider-rs ELF preflight failed");
                return report;
            }
        }

        match kernel.bootstrap_spider_rs_pid1_via_mtss(image) {
            Ok(pid) => {
                report.pid = Some(pid);
                report.process_created = true;
                report.stack_preflight_ok = true;
                report.handoff_state = SpiderPid1HandoffState::ProcessCreated;
            }
            Err(KernelError::Loader(LoadError::StackBuildFailed)) => {
                report.handoff_state =
                    SpiderPid1HandoffState::Failed(SpiderPid1HandoffError::StackAllocationFailed);
                report.launch_blocker = Some("PID1 stack preflight failed");
                return report;
            }
            Err(_) => {
                report.handoff_state =
                    SpiderPid1HandoffState::Failed(SpiderPid1HandoffError::ProcessCreateFailed);
                report.launch_blocker = Some("PID1 process creation failed");
                return report;
            }
        }

        if let Some(task) = kernel.mtss_pid1_task() {
            report.mtss_task_id = Some(task.id);
        } else {
            report.launch_blocker = Some("MTSS PID1 task was not created");
            return report;
        }
        if let Some(thread) = kernel.mtss_pid1_main_thread() {
            report.mtss_thread_id = Some(thread.id);
            report.main_thread_created = true;
            report.accepted_into_run_queue = matches!(
                thread.state,
                mirage_mtss::ThreadState::Ready | mirage_mtss::ThreadState::Running
            );
        } else {
            report.launch_blocker = Some("MTSS PID1 main thread was not created");
            return report;
        }
        if !report.accepted_into_run_queue {
            report.launch_blocker = Some("MTSS did not accept PID1 into the run queue");
            return report;
        }
        report.admitted_through_mtss = true;
        report.handoff_state = SpiderPid1HandoffState::Runnable;
        report.dispatcher_state =
            SpiderDispatcherState::Pending("user-mode transition not implemented");
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::Kernel;
    use mirage_mtss::{CoreTaskId, TaskKind, TaskState, ThreadState};

    fn minimal_spider_elf() -> [u8; 128] {
        let mut image = [0u8; 128];
        image[0..4].copy_from_slice(b"\x7fELF");
        image[4] = 2;
        image[5] = 1;
        image[16..18].copy_from_slice(&2u16.to_le_bytes());
        image[18..20].copy_from_slice(&62u16.to_le_bytes());
        image[24..32].copy_from_slice(&0x401000u64.to_le_bytes());
        image[32..40].copy_from_slice(&64u64.to_le_bytes());
        image[54..56].copy_from_slice(&56u16.to_le_bytes());
        image[56..58].copy_from_slice(&1u16.to_le_bytes());
        image[64..68].copy_from_slice(&1u32.to_le_bytes());
        image[68..72].copy_from_slice(&5u32.to_le_bytes());
        image[72..80].copy_from_slice(&0u64.to_le_bytes());
        image[80..88].copy_from_slice(&0x400000u64.to_le_bytes());
        image[96..104].copy_from_slice(&16u64.to_le_bytes());
        image[104..112].copy_from_slice(&0x2000u64.to_le_bytes());
        image
    }

    #[test]
    fn spider_pid1_is_authorized_by_supervisor_and_admitted_by_mtss() {
        let mut kernel: Kernel<16, 16> = Kernel::new();
        kernel.bootstrap();
        kernel.kernel_mtss_init().expect("MTSS init should succeed");
        let supervisor = Supervisor::new();

        let image = minimal_spider_elf();
        let report = supervisor
            .launch_spider_rs_pid1_via_mtss(&mut kernel, &image)
            .expect("supervisor-authorized Spider-rs image should enter MTSS");

        assert_eq!(
            report.pid.expect("pid created").raw(),
            CoreTaskId::FIRST_USERSPACE.raw()
        );
        assert_eq!(report.runtime_path, SPIDER_RS_RUNTIME_PATH);
        assert!(report.authorized_by_supervisor);
        assert!(report.admitted_through_mtss);
        assert!(report.process_created);
        assert!(report.main_thread_created);
        assert!(report.entry_preflight_ok);
        assert!(report.stack_preflight_ok);
        assert_eq!(report.mtss_task_id, Some(CoreTaskId::FIRST_USERSPACE));
        assert_eq!(
            report.mtss_thread_id,
            Some(mirage_mtss::CoreThreadId::new(1))
        );
        assert!(report.accepted_into_run_queue);

        let task = kernel.mtss_pid1_task().expect("pid1 is visible to MTSS");
        assert_eq!(task.id, CoreTaskId::FIRST_USERSPACE);
        assert_eq!(task.kind, TaskKind::Userspace);
        assert_eq!(task.state, TaskState::Runnable);
        assert_eq!(task.name, "spider-rs");

        let thread = kernel
            .mtss_pid1_main_thread()
            .expect("pid1 main thread is visible to MTSS");
        assert_eq!(thread.task, CoreTaskId::FIRST_USERSPACE);
        assert_eq!(thread.state, ThreadState::Ready);
        assert_eq!(thread.context.rip, 0x401000);
    }
}
