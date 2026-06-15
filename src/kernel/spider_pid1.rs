//! MTSS-facing Spider-rs PID 1 admission shim.
//!
//! This file deliberately does not make scheduling policy decisions.  It gives
//! the Supervisor one clean kernel entry point whose name makes the intended
//! authority chain explicit: Supervisor authorizes, kernel/userspace loader
//! validates/maps, MTSS admits the task, and the architecture backend performs
//! the eventual ring-3 entry.

use crate::kernel::process::ProcessId;
use crate::kernel::{Kernel, KernelError};

impl<const NPROC: usize, const MSG_DEPTH: usize> Kernel<NPROC, MSG_DEPTH> {
    /// Admit Spider-rs as the initial userspace task through the MTSS-owned PID 1 path.
    ///
    /// The existing `bootstrap_spider_rs_pid1_from_image` path is kept as the
    /// single mechanical loader/admission implementation.  This wrapper exists
    /// so callers no longer bypass the Supervisor->MTSS handoff language in
    /// boot code.
    pub fn bootstrap_spider_rs_pid1_via_mtss(
        &mut self,
        image: &[u8],
    ) -> Result<ProcessId, KernelError> {
        self.bootstrap_spider_rs_pid1_from_image(image)
    }
}
