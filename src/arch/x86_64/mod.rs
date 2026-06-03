//! 64-bit x86 bootstrap support layer.
//!
//! This module owns the processor-facing initialization sequence before Mirage hands
//! control to higher-level kernel subsystems.

use core::hint::spin_loop;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::arch::x86_64::boot::BootInfo;
use crate::kernel::memory;
use crate::kernel::syscall::{SyscallFrame, SYSCALL_MAX_ARGS};
use crate::kernel::thread::{
    CpuContext, ThreadControlBlock, ThreadId, SYSCALL_TRAP_VECTOR, TIMER_INTERRUPT_VECTOR,
};

pub mod boot;
pub mod clock;
pub mod device;
pub mod gdt;
pub mod idt;
pub mod interrupts;
pub mod io;
pub mod limine_block;
pub mod msr;
pub mod paging;
pub mod pic;
pub mod ps2_keyboard;
pub mod uart16550;

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

/// Version of the internal assembly/Rust CpuContext frame contract.
///
/// Keep this at 1 while `entry.S` stores fields in exactly the same order as
/// `kernel::thread::CpuContext`; bump it if a future frame layout intentionally
/// changes and all users are migrated together.
pub const CPU_CONTEXT_ABI_VERSION: u64 = 1;

#[no_mangle]
pub static __mirage_current_core: AtomicUsize = AtomicUsize::new(usize::MAX);
#[no_mangle]
pub static __mirage_current_thread: AtomicU64 = AtomicU64::new(0);
static CURRENT_CONTEXT: AtomicUsize = AtomicUsize::new(0);

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
pub fn run_thread_slice(core_index: usize, thread: &mut ThreadControlBlock) -> ThreadRunOutcome {
    let timer_epoch = idt::timer_ticks();

    enter_thread_slice(core_index, thread);

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
    fn __mirage_context_restore(context: *mut crate::kernel::thread::CpuContext);
}

/// Restore the saved CPU context for a thread.
///
/// On hardware this returns only after timer preemption or syscall trap: `__mirage_context_restore` rebuilds
/// the CPU's interrupt-return frame and executes `iretq`. The interrupt and
/// syscall stubs save the next frame before re-entering Rust scheduler code.
pub fn switch_to_thread(thread: &mut ThreadControlBlock) {
    enter_thread_slice(0, thread);
}

/// Publish the current hardware scheduler identity and restore a thread frame.
///
/// Interrupt and syscall assembly reads these atomics when it builds a
/// [`CpuContext`] trap frame, then calls back into Rust to copy that frame into
/// the running [`ThreadControlBlock`].
pub fn enter_thread_slice(core_index: usize, thread: &mut ThreadControlBlock) {
    __mirage_current_core.store(core_index, Ordering::SeqCst);
    __mirage_current_thread.store(thread.id.raw(), Ordering::SeqCst);
    CURRENT_CONTEXT.store(
        core::ptr::addr_of_mut!(thread.context) as usize,
        Ordering::SeqCst,
    );

    #[cfg(not(test))]
    unsafe {
        __mirage_context_restore(core::ptr::addr_of_mut!(thread.context));
    }

    #[cfg(test)]
    {
        let _ = thread;
    }

    CURRENT_CONTEXT.store(0, Ordering::SeqCst);
    __mirage_current_thread.store(0, Ordering::SeqCst);
    __mirage_current_core.store(usize::MAX, Ordering::SeqCst);
}

/// Rust callback used by x86_64 trap entry to persist the hardware frame.
#[no_mangle]
pub extern "C" fn __mirage_arch_save_trap_frame(
    frame: *const CpuContext,
    core_index: usize,
    thread_raw: u64,
) {
    let context_ptr = CURRENT_CONTEXT.load(Ordering::SeqCst) as *mut CpuContext;
    if !frame.is_null() && !context_ptr.is_null() {
        unsafe {
            *context_ptr = *frame;
        }
    }

    let saved = unsafe { frame.as_ref() };
    let (vector, error_code) = saved
        .map(|context| (context.trap_vector, context.error_code))
        .unwrap_or((0, 0));
    let _ = (core_index, thread_raw, CPU_CONTEXT_ABI_VERSION);
    idt::dispatch_interrupt(vector, error_code);
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
    memory::initialize_from_boot_info(boot_info);
}

fn configure_interrupts() {
    idt::initialize();
    pic::initialize();
    interrupts::enable();
}
