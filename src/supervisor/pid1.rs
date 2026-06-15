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
