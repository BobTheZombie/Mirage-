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
pub const M1_TERMINAL_PATH: &str = "/spider-rt/bin/mirage-m1-terminal";
pub const M1_TERMINAL_OUTPUT: &str = "Mirage M1.1 System\nhello world\n";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SpiderPid1Preconditions {
    pub root_fs_online: bool,
    pub runtime_vfs_mounted: bool,
    pub spider_binary_present: bool,
    pub mtss_online: bool,
    pub userspace_loader_ready: bool,
}

/// Successful Supervisor -> MTSS PID 1 handoff record.
#[derive(Clone, Copy, Debug)]
pub struct SpiderPid1LaunchReport {
    pub pid: ProcessId,
    pub task_id: mirage_mtss::CoreTaskId,
    pub thread_id: mirage_mtss::CoreThreadId,
    pub entry: VirtAddr,
    pub image_len: usize,
    pub runtime_path: &'static str,
    pub authorized_by_supervisor: bool,
    pub admitted_through_mtss: bool,
    pub handoff_state: SpiderPid1HandoffState,
    pub dispatcher_state: SpiderDispatcherState,
    pub terminal_manifest: TerminalManifestEntry,
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
        self.launch_spider_rs_pid1_checked(
            kernel,
            image,
            SpiderPid1Preconditions {
                root_fs_online: true,
                runtime_vfs_mounted: true,
                spider_binary_present: !image.is_empty(),
                mtss_online: true,
                userspace_loader_ready: true,
            },
        )
    }

    pub fn launch_spider_rs_pid1_checked<const NPROC: usize, const MSG_DEPTH: usize>(
        &self,
        kernel: &mut Kernel<NPROC, MSG_DEPTH>,
        image: &[u8],
        preconditions: SpiderPid1Preconditions,
    ) -> Result<SpiderPid1LaunchReport, SpiderPid1LaunchError> {
        let _ = self;
        if !preconditions.root_fs_online
            || !preconditions.runtime_vfs_mounted
            || !preconditions.spider_binary_present
            || !preconditions.mtss_online
            || !preconditions.userspace_loader_ready
        {
            return Err(SpiderPid1LaunchError::Handoff(
                SpiderPid1HandoffError::SupervisorDenied,
            ));
        }
        if image.is_empty() {
            return Err(SpiderPid1LaunchError::Handoff(
                SpiderPid1HandoffError::SpiderBinaryMissing,
            ));
        }

        let entry = validate_elf64(image).map_err(|error| match error {
            LoadError::UnsupportedMachine => {
                SpiderPid1LaunchError::Handoff(SpiderPid1HandoffError::UnsupportedElfArch)
            }
            other => SpiderPid1LaunchError::Load(other),
        })?;
        let pid = kernel
            .bootstrap_spider_rs_pid1_via_mtss(image)
            .map_err(SpiderPid1LaunchError::Kernel)?;
        let task = kernel
            .mtss_pid1_task()
            .ok_or(SpiderPid1LaunchError::Handoff(
                SpiderPid1HandoffError::MtssSpawnFailed,
            ))?;
        let thread = kernel
            .mtss_pid1_main_thread()
            .ok_or(SpiderPid1LaunchError::Handoff(
                SpiderPid1HandoffError::MtssSpawnFailed,
            ))?;

        Ok(SpiderPid1LaunchReport {
            pid,
            task_id: task.id,
            thread_id: thread.id,
            entry,
            image_len: image.len(),
            runtime_path: SPIDER_RS_RUNTIME_PATH,
            authorized_by_supervisor: true,
            admitted_through_mtss: true,
            handoff_state: SpiderPid1HandoffState::Runnable,
            dispatcher_state: SpiderDispatcherState::Pending(
                "user-mode transition not implemented",
            ),
            terminal_manifest: TerminalManifestEntry {
                path: M1_TERMINAL_PATH,
                expected_output: M1_TERMINAL_OUTPUT,
                state: TerminalLaunchState::PendingDispatcherChildLaunch,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::Kernel;
    use mirage_mtss::{CoreTaskId, CoreTaskState, TaskKind};

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
        kernel.kernel_mtss_init();
        let supervisor = Supervisor::new();

        let image = minimal_spider_elf();
        let report = supervisor
            .launch_spider_rs_pid1_via_mtss(&mut kernel, &image)
            .expect("supervisor-authorized Spider-rs image should enter MTSS");

        assert_eq!(report.pid.raw(), CoreTaskId::FIRST_USERSPACE.raw());
        assert_eq!(report.runtime_path, SPIDER_RS_RUNTIME_PATH);
        assert!(report.authorized_by_supervisor);
        assert!(report.admitted_through_mtss);

        let task = kernel.mtss_pid1_task().expect("pid1 is visible to MTSS");
        assert_eq!(task.id, CoreTaskId::FIRST_USERSPACE);
        assert_eq!(task.kind, TaskKind::Userspace);
        assert_eq!(task.state, CoreTaskState::Ready);
        assert_eq!(task.name, "spider-rs");

        let thread = kernel
            .mtss_pid1_main_thread()
            .expect("pid1 main thread is visible to MTSS");
        assert_eq!(thread.task, CoreTaskId::FIRST_USERSPACE);
        assert_eq!(thread.state, CoreTaskState::Ready);
        assert_eq!(thread.context.rip, 0x401000);
    }
}
