//! Minimal 64-bit x86 support layer.
//!
//! The routines in this module intentionally avoid touching real hardware. They provide
//! structural placeholders that outline how a Rust kernel would prepare the processor
//! before handing over control to the higher level subsystems.

use core::hint::spin_loop;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::x86_64::boot::BootInfo;
use crate::kernel::syscall::SYSCALL_MAX_ARGS;
use crate::kernel::thread::{ThreadControlBlock, ThreadId};

pub mod boot;
pub mod clock;

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
/// In a full kernel this would configure control registers, descriptor tables, and paging.
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
    loop {
        cpu_relax();
    }
}

fn configure_cpu_modes() {
    // Placeholder for enabling long mode, installing the GDT/IDT, and switching privilege levels.
}

fn setup_memory_layout(_boot_info: &BootInfo) {
    // A real kernel would use the parsed memory map, direct-map offset, and kernel section ranges
    // to establish page ownership and protect text/rodata. Mirage keeps this as a structural hook
    // until the physical memory manager and page tables grow real hardware implementations.
}

fn configure_interrupts() {
    // In the conceptual design we defer actual interrupt controller programming, but the hook is
    // left in place so higher layers can request timer ticks or I/O notifications later.
}
