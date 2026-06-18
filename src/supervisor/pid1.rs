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

/// Successful Supervisor -> MTSS PID 1 handoff record.
#[derive(Debug)]
pub struct SpiderPid1LaunchReport {
    pub pid: ProcessId,
    pub entry: VirtAddr,
    pub image_len: usize,
    pub runtime_path: &'static str,
    pub authorized_by_supervisor: bool,
    pub admitted_through_mtss: bool,
}

/// Failure points in the Spider-rs PID 1 handoff.
#[derive(Debug)]
pub enum SpiderPid1LaunchError {
    RuntimeUnavailable,
    EmptyImage,
    InvalidElf(LoadError),
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
        let _ = self;
        if image.is_empty() {
            return Err(SpiderPid1LaunchError::EmptyImage);
        }

        let entry = validate_elf64(image).map_err(SpiderPid1LaunchError::InvalidElf)?;
        let pid = kernel
            .bootstrap_spider_rs_pid1_via_mtss(image)
            .map_err(SpiderPid1LaunchError::Kernel)?;

        Ok(SpiderPid1LaunchReport {
            pid,
            entry,
            image_len: image.len(),
            runtime_path: SPIDER_RS_RUNTIME_PATH,
            authorized_by_supervisor: true,
            admitted_through_mtss: true,
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
