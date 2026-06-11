//! 64-bit x86 bootstrap support layer.
//!
//! This module owns the processor-facing initialization sequence before Mirage hands
//! control to higher-level kernel subsystems.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::arch::x86_64::boot::BootInfo;
use crate::arch::x86_64::ps2_keyboard::PS2_KEYBOARD_DRIVER;
use crate::kernel::cpu::MAX_CORES;
use crate::kernel::device::{DeviceDriver, MirageInputEvent};
#[cfg(not(feature = "emergency-boot"))]
use crate::kernel::memory;
use crate::kernel::syscall::{SyscallFrame, SYSCALL_MAX_ARGS};
use crate::kernel::thread::{
    CpuContext, ThreadControlBlock, ThreadId, SYSCALL_TRAP_VECTOR, TIMER_INTERRUPT_VECTOR,
};

pub mod boot;
pub mod clock;
pub mod device;
pub mod early_console;
pub mod early_debug;
#[cfg(feature = "hw-framebuffer")]
pub mod framebuffer_console;
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

#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub struct PerCpuState {
    pub kernel_stack_top: u64,
    pub user_rsp: u64,
}

impl PerCpuState {
    pub const fn new() -> Self {
        Self {
            kernel_stack_top: 0,
            user_rsp: 0,
        }
    }
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
static mut PER_CPU: [PerCpuState; MAX_CORES] = [PerCpuState::new(); MAX_CORES];

/// Perform one-time CPU and memory initialisation.
///
/// Normal boots install descriptor tables, early paging, framebuffer console
/// state, and interrupt controller state. Emergency boots deliberately stop
/// after raw serial diagnostics so the halt path cannot reach heap, paging,
/// framebuffer, or interrupt-controller setup before those subsystems are safe.
pub fn init_architecture(boot_info: &BootInfo) {
    if INITIALISED.swap(true, Ordering::SeqCst) {
        return;
    }

    uart16550::init_early_serial();

    #[cfg(feature = "emergency-boot")]
    {
        let _ = boot_info;
        unsafe {
            early_debug::com1_write_str("serial initialized\r\n");
        }
    }

    #[cfg(not(feature = "emergency-boot"))]
    {
        crate::kprintln!("serial initialized");
        configure_cpu_modes();
        initialize_per_cpu_state();
        setup_memory_layout(boot_info);
        initialize_framebuffer_console(boot_info);
        configure_interrupts();
    }
}

#[cfg(all(not(feature = "emergency-boot"), feature = "hw-framebuffer"))]
fn initialize_framebuffer_console(boot_info: &BootInfo) {
    match framebuffer_console::init_from_boot_info(boot_info) {
        Ok(Some(framebuffer)) => {
            crate::kprintln!("Mirage framebuffer online");
            crate::kprintln!("  resolution: {}x{}", framebuffer.width, framebuffer.height);
            crate::kprintln!("  pitch: {}", framebuffer.pitch);
            crate::kprintln!("  bits per pixel: {}", framebuffer.bits_per_pixel);
            crate::kprintln!("  framebuffer address: {:#018x}", framebuffer.address.0);
        }
        Ok(None) | Err(_) => {
            crate::kprintln!("framebuffer unavailable; serial console only");
        }
    }
}

#[cfg(all(not(feature = "emergency-boot"), not(feature = "hw-framebuffer")))]
fn initialize_framebuffer_console(_boot_info: &BootInfo) {
    crate::kprintln!("framebuffer unavailable; serial console only");
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
    prepare_core_entry_state(core_index);

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
    let _ = (core_index, thread_raw, CPU_CONTEXT_ABI_VERSION);
    if let Some(context) = saved {
        idt::dispatch_interrupt_frame(context);
    } else {
        idt::dispatch_interrupt(0, 0);
    }
}

pub fn kernel_stack_top(core_index: usize) -> u64 {
    gdt::kernel_stack_top(core_index)
}

pub fn per_cpu_state_ptr(core_index: usize) -> u64 {
    let index = if core_index < MAX_CORES {
        core_index
    } else {
        0
    };
    unsafe { core::ptr::addr_of!(PER_CPU[index]) as u64 }
}

#[cfg(not(feature = "emergency-boot"))]
fn initialize_per_cpu_state() {
    let mut idx = 0usize;
    while idx < MAX_CORES {
        unsafe {
            PER_CPU[idx].kernel_stack_top = gdt::kernel_stack_top(idx);
            PER_CPU[idx].user_rsp = 0;
        }
        idx += 1;
    }
    prepare_core_entry_state(0);
}

fn prepare_core_entry_state(core_index: usize) {
    let index = if core_index < MAX_CORES {
        core_index
    } else {
        0
    };
    unsafe {
        if PER_CPU[index].kernel_stack_top == 0 {
            PER_CPU[index].kernel_stack_top = gdt::kernel_stack_top(index);
        }
    }
    gdt::set_current_kernel_stack(index);
    msr::write_gs_base(per_cpu_state_ptr(index));
    msr::write_kernel_gs_base(per_cpu_state_ptr(index));
}

/// Return the number of hardware timer interrupts observed by the architecture layer.
pub fn timer_ticks() -> u64 {
    idt::timer_ticks()
}

/// Report whether a new hardware timer tick needs kernel-level dispatch.
pub fn timer_tick_pending(last_observed_tick: &mut u64) -> bool {
    let current_tick = timer_ticks();
    if current_tick != *last_observed_tick {
        *last_observed_tick = current_tick;
        true
    } else {
        false
    }
}

/// Poll the early PS/2 keyboard path for the ESC debug-shell hotkey.
pub fn poll_debug_shell_hotkey() -> bool {
    const EVENT_CAPACITY: usize = 4;
    let event_size = core::mem::size_of::<MirageInputEvent>();
    let mut buffer = [0u8; core::mem::size_of::<MirageInputEvent>() * EVENT_CAPACITY];

    let Ok(byte_count) = PS2_KEYBOARD_DRIVER.read(&mut buffer) else {
        return false;
    };
    if byte_count == 0 || byte_count % event_size != 0 {
        return false;
    }

    let mut offset = 0usize;
    while offset < byte_count {
        if let Some(event) = decode_input_event(&buffer[offset..offset + event_size]) {
            if event.event_type == 1 && event.code == 0x01 && event.value == 1 {
                return true;
            }
        } else {
            return false;
        }
        offset += event_size;
    }

    false
}

fn decode_input_event(bytes: &[u8]) -> Option<MirageInputEvent> {
    if bytes.len() != core::mem::size_of::<MirageInputEvent>() {
        return None;
    }

    Some(MirageInputEvent {
        event_type: u16::from_ne_bytes([bytes[0], bytes[1]]),
        code: u16::from_ne_bytes([bytes[2], bytes[3]]),
        value: i32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        timestamp_ns: u64::from_ne_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        ]),
    })
}

/// Halt the CPU while the boot core is idle until the next interrupt arrives.
#[inline(always)]
pub fn idle_halt() {
    interrupts::halt();
}

/// Hint to the CPU that the current core is in a spin loop.
#[inline(always)]
pub fn cpu_relax() {
    core::hint::spin_loop();
}

/// Halt the CPU after panic diagnostics are written to COM1.
///
/// This is the final architecture-specific panic path: maskable interrupts are
/// disabled before the CPU enters an infinite `hlt` loop, so no scheduler or IRQ
/// policy runs after the panic output has been emitted. In a real system an IPI
/// or watchdog would reset us.
pub fn panic_halt() -> ! {
    interrupts::halt_forever()
}

#[cfg(not(feature = "emergency-boot"))]
fn configure_cpu_modes() {
    interrupts::disable();
    gdt::initialize();
    crate::kprintln!("GDT initialized");
}

#[cfg(not(feature = "emergency-boot"))]
fn setup_memory_layout(boot_info: &BootInfo) {
    validate_boot_memory_handoff(boot_info);

    paging::initialize(boot_info);
    memory::initialize_from_boot_info(boot_info);
    crate::kprintln!("memory map parsed");
    crate::kprintln!("memory initialized");
    print_early_memory_diagnostics(boot_info);
}

#[cfg(not(feature = "emergency-boot"))]
fn validate_boot_memory_handoff(boot_info: &BootInfo) {
    let mut fatal = false;

    if boot_info.memory_map.is_none() {
        crate::kprintln!("FATAL: boot memory map is missing; cannot initialize physical memory");
        fatal = true;
    }

    if boot_info.hhdm_offset.is_none() {
        crate::kprintln!("FATAL: HHDM offset is missing; cannot translate bootloader mappings");
        fatal = true;
    }

    if fatal {
        crate::kprintln!("FATAL: Mirage requires a complete Limine memory handoff; halting");
        panic_halt();
    }
}

#[cfg(not(feature = "emergency-boot"))]
fn print_early_memory_diagnostics(boot_info: &BootInfo) {
    let translator = paging::AddressTranslator::new(boot_info);
    let physical = memory::physical_stats();
    let heap = memory::heap_stats();

    crate::kprintln!("early memory diagnostics:");

    if let Some(hhdm_offset) = boot_info.hhdm_offset {
        crate::kprintln!("  HHDM offset: {:#018x}", hhdm_offset);
    }

    if let Some(load) = boot_info.kernel.load_range {
        let physical_end = load.physical_start.0.saturating_add(load.length);
        let virtual_end = load.virtual_start.0.saturating_add(load.length);
        crate::kprintln!(
            "  kernel physical: {:#018x}..{:#018x}",
            load.physical_start.0,
            physical_end
        );
        crate::kprintln!(
            "  kernel virtual:  {:#018x}..{:#018x}",
            load.virtual_start.0,
            virtual_end
        );
    } else {
        crate::kprintln!("  kernel load range: unavailable from bootloader");
    }

    if let Some(framebuffer) = boot_info.framebuffer {
        let framebuffer_len = framebuffer.pitch.saturating_mul(framebuffer.height);
        let framebuffer_start = translator.physical_for_virtual(framebuffer.address.0);
        let framebuffer_end = framebuffer_start.saturating_add(framebuffer_len);
        crate::kprintln!(
            "  framebuffer physical: {:#018x}..{:#018x} ({} bytes)",
            framebuffer_start,
            framebuffer_end,
            framebuffer_len
        );
    } else {
        crate::kprintln!("  framebuffer physical: none");
    }

    crate::kprintln!(
        "  boot modules: count={}, reserved={} bytes",
        boot_info.modules.len(),
        physical.module_bytes
    );
    crate::kprintln!(
        "  allocator metadata physical: {:#018x}..{:#018x} ({} bytes)",
        physical.metadata_physical_start,
        physical.metadata_physical_end,
        physical.metadata_bytes
    );
    crate::kprintln!(
        "  memory bytes: total_map={}, usable={}, reserved={}, bootloader_reclaimable={}, acpi={}, mmio_framebuffer={}, kernel_module={}",
        physical.total_memory_map_bytes,
        physical.usable_bytes,
        physical.reserved_bytes,
        physical.bootloader_reclaimable_bytes,
        physical.acpi_bytes,
        physical.mmio_framebuffer_bytes,
        physical.kernel_module_bytes
    );
    crate::kprintln!(
        "  frames: total={}, free={}, used={}",
        physical.total_frame_count,
        physical.free_frame_count,
        physical.used_frame_count
    );
    crate::kprintln!(
        "  heap: {:#018x}..{:#018x}, committed={} bytes, reserved={} bytes",
        heap.base,
        heap.end,
        heap.committed_bytes,
        heap.reserved_bytes
    );
}

#[cfg(not(feature = "emergency-boot"))]
fn configure_interrupts() {
    idt::initialize();
    crate::kprintln!("IDT initialized");
    pic::initialize();
    crate::kprintln!("PIC initialized");
    interrupts::enable();
    crate::kprintln!("interrupts enabled");
}
