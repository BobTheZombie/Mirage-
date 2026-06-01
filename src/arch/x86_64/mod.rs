//! 64-bit x86 bootstrap support layer.
//!
//! This module owns the processor-facing initialization sequence before Mirage hands
//! control to higher-level kernel subsystems.

use core::hint::spin_loop;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::x86_64::boot::BootInfo;
use crate::kernel::syscall::{SyscallFrame, SYSCALL_MAX_ARGS};
use crate::kernel::thread::{
    ThreadControlBlock, ThreadId, SYSCALL_TRAP_VECTOR, TIMER_INTERRUPT_VECTOR,
};

pub mod boot;
pub mod clock;
pub mod gdt;
pub mod idt;
pub mod interrupts;
pub mod msr;
pub mod paging;
pub mod pic;

pub use clock::{HardwareClock, HARDWARE_CLOCK};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallTrap {
    pub thread: ThreadId,
    pub number: u64,
    pub args: [u64; SYSCALL_MAX_ARGS],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadRunOutcome {
    TimeSliceComplete,
    TimerPreempted,
    Syscall(SyscallTrap),
}

static INITIALISED: AtomicBool = AtomicBool::new(false);

/// Perform one-time CPU and memory initialisation.
///
/// Install descriptor tables, early paging, and interrupt controller state.
pub fn init_architecture(boot_info: &BootInfo) {
    if INITIALISED.swap(true, Ordering::SeqCst) {
        return;
    }

    configure_cpu_modes();
    setup_memory_layout(boot_info);
    configure_interrupts();
}

/// Run a scheduled thread until hardware returns control through a trap.
///
/// The x86_64 path restores the thread's saved interrupt frame and returns to
/// the privilege level captured in [`CpuContext`](crate::kernel::thread::CpuContext).
/// Control comes back only after an interrupt or syscall entry stub saves a new
/// frame in the same context. Unit tests use the same register ABI by staging a
/// trap frame in the thread context before invoking the scheduler.
pub fn run_thread_slice(thread: &mut ThreadControlBlock) -> ThreadRunOutcome {
    let timer_epoch = idt::timer_ticks();

    switch_to_thread(thread);

    match thread.context.trap_vector {
        SYSCALL_TRAP_VECTOR => ThreadRunOutcome::Syscall(SyscallTrap {
            thread: thread.id,
            number: SyscallFrame::from_cpu_context(&thread.context).number,
            args: SyscallFrame::from_cpu_context(&thread.context).args,
        }),
        TIMER_INTERRUPT_VECTOR => {
            thread.context.clear_trap();
            ThreadRunOutcome::TimerPreempted
        }
        _ if idt::timer_ticks() != timer_epoch => ThreadRunOutcome::TimerPreempted,
        _ => ThreadRunOutcome::TimeSliceComplete,
    }
}

#[cfg(not(test))]
extern "C" {
    fn __mirage_context_restore(context: *mut crate::kernel::thread::CpuContext) -> !;
}

/// Restore the saved CPU context for a thread.
///
/// On hardware this never returns directly: `__mirage_context_restore` rebuilds
/// the CPU's interrupt-return frame and executes `iretq`. The interrupt and
/// syscall stubs save the next frame before re-entering Rust scheduler code.
pub fn switch_to_thread(thread: &mut ThreadControlBlock) {
    #[cfg(not(test))]
    unsafe {
        __mirage_context_restore(core::ptr::addr_of_mut!(thread.context));
    }

    #[cfg(test)]
    {
        let _ = thread;
    }
}

/// Hint to the CPU that the current core is in a spin loop.
#[inline(always)]
pub fn cpu_relax() {
    spin_loop();
}

/// Halt the CPU in a panic scenario. In a real system an IPI or watchdog would reset us.
pub fn panic_halt() -> ! {
    interrupts::halt_forever()
}

fn configure_cpu_modes() {
    interrupts::disable();
    gdt::initialize();
}

fn setup_memory_layout(boot_info: &BootInfo) {
    paging::initialize(boot_info);
}

fn configure_interrupts() {
    idt::initialize();
    pic::initialize();
    interrupts::enable();
}
