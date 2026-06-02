//! Clock and timer service seam.

use crate::kernel::process::ProcessId;
use crate::kernel::syscall::{SyscallContext, SyscallNumber};
use crate::kernel::thread::ThreadId;
use crate::kernel::time::{MonotonicTimestamp, KERNEL_TIME};
use crate::kernel::{Kernel, KernelResult, MirageTimespec};

/// Kernel-internal adapter for clock and timer operations.
pub trait TimeService {
    fn tick(&mut self);

    fn clock_gettime(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        clock_id: i32,
        out: &mut MirageTimespec,
    ) -> KernelResult<()>;

    fn monotonic_now(&self) -> MonotonicTimestamp;
}

impl<const MAX_PROC: usize, const MSG_DEPTH: usize> TimeService for Kernel<MAX_PROC, MSG_DEPTH> {
    fn tick(&mut self) {
        Kernel::tick(self);
    }

    fn clock_gettime(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        clock_id: i32,
        out: &mut MirageTimespec,
    ) -> KernelResult<()> {
        self.handle_syscall(
            SyscallNumber::ClockGettime.raw(),
            SyscallContext::new(
                caller,
                thread,
                [
                    clock_id as u64,
                    out as *mut MirageTimespec as u64,
                    0,
                    0,
                    0,
                    0,
                ],
            ),
        )
        .map(|_| ())
    }

    fn monotonic_now(&self) -> MonotonicTimestamp {
        KERNEL_TIME.now()
    }
}
