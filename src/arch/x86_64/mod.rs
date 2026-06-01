//! 64-bit x86 bootstrap support layer.
//!
//! This module owns the processor-facing initialization sequence before Mirage hands
//! control to higher-level kernel subsystems.

use core::hint::spin_loop;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::x86_64::boot::BootInfo;
use crate::kernel::syscall::SYSCALL_MAX_ARGS;
use crate::kernel::thread::{ThreadControlBlock, ThreadId};

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

/// Run a scheduled thread until its simulated time slice expires or it traps.
///
/// A real x86_64 port would restore the saved register frame with `iretq`/`sysret`
/// and regain control through an interrupt or `syscall` entry stub. Mirage keeps
/// that machinery explicit in the saved context: tests and libc shims can queue
/// a syscall in the thread context, this arch layer observes the trap, and the
/// kernel writes the return register before the thread is requeued.
pub fn run_thread_slice(thread: &mut ThreadControlBlock) -> ThreadRunOutcome {
    switch_to_thread(thread);

    if let Some((number, args)) = thread.context.take_syscall() {
        ThreadRunOutcome::Syscall(SyscallTrap {
            thread: thread.id,
            number,
            args,
        })
    } else {
        ThreadRunOutcome::TimeSliceComplete
    }
}

/// Restore the saved CPU context for a thread.
///
/// This is intentionally a no-op in the simulator, but it marks the ABI boundary
/// where an x86_64 implementation would load RIP/RSP/RFLAGS and general-purpose
/// registers before entering user mode.
pub fn switch_to_thread(_thread: &mut ThreadControlBlock) {}

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
