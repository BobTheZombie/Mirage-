//! 64-bit x86 bootstrap support layer.
//!
//! This module owns the processor-facing initialization sequence before Mirage hands
//! control to higher-level kernel subsystems.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
#[cfg(not(feature = "emergency-boot"))]
use mirage_platform::PlatformPciBar;

#[cfg(feature = "hw-laptop-hotkeys")]
use crate::arch::x86_64::acpi_ec::{AcpiEcStatus, ACPI_EC_HOTKEY_DRIVER};
use crate::arch::x86_64::boot::BootInfo;
#[cfg(feature = "hw-ps2-keyboard")]
use crate::arch::x86_64::ps2_keyboard::PS2_KEYBOARD_DRIVER;
#[cfg(feature = "hw-usb-hid")]
use crate::arch::x86_64::xhci_keyboard::{
    DriverStatus, XhciKeyboardStatus, USB_HID_KEYBOARD_DRIVER,
};
use crate::kernel::boot_phase::{
    boot_phase_detected, boot_phase_failed, boot_phase_ok, boot_phase_online, boot_phase_skipped,
    boot_phase_start, boot_phase_stub, BootPhase,
};
use crate::kernel::cpu::MAX_CORES;
#[cfg(not(feature = "emergency-boot"))]
use crate::kernel::memory;
use crate::kernel::process::ProcessId;
use crate::kernel::syscall::{SyscallFrame, SYSCALL_MAX_ARGS};
use crate::kernel::thread::{CpuContext, ThreadId, SYSCALL_TRAP_VECTOR, TIMER_INTERRUPT_VECTOR};

#[cfg(feature = "hw-laptop-hotkeys")]
pub mod acpi_ec;
pub mod acpi_madt;
#[cfg(feature = "hw-ahci")]
pub mod ahci;
pub mod apic;
pub mod boot;
pub mod clock;
pub mod device;
pub mod early_console;
pub mod early_debug;
#[cfg(feature = "hw-framebuffer")]
pub mod framebuffer_console;
pub mod gdt;
#[cfg(feature = "hw-i8042")]
pub mod i8042;
pub mod idt;
pub mod interrupts;
pub mod io;
pub mod ioapic;
pub mod irq;
pub mod limine_block;
pub mod msr;
pub mod paging;
pub mod platform;

pub mod pic;
#[cfg(feature = "hw-ps2-keyboard")]
pub mod ps2_keyboard;
#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
pub mod seed_rs;
pub mod uart16550;
#[cfg(feature = "hw-usb-hid")]
pub mod xhci_keyboard;

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
    UserEntryInvalid,
    Syscall(SyscallTrap),
}

/// Explicit kernel-to-architecture handoff for a single MTSS-selected slice.
///
/// The kernel scheduler constructs this only after MTSS selects a runnable
/// thread. The architecture layer consumes the already-made policy decision and
/// performs only CPU mechanism: address-space switch, TSS.rsp0 update, frame
/// validation, and context restore.
pub struct ThreadSliceRunContext<'a> {
    pub core_index: usize,
    pub thread: ThreadId,
    pub process: ProcessId,
    pub address_space_root: u64,
    pub kernel_stack_top: u64,
    pub context: &'a mut CpuContext,
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

    #[cfg(not(feature = "emergency-boot"))]
    boot_phase_start(BootPhase::Serial);
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
        boot_phase_ok(BootPhase::Serial);
        configure_cpu_modes();
        initialize_per_cpu_state();
        setup_memory_layout(boot_info);
        initialize_framebuffer_console(boot_info);
        configure_interrupts(boot_info);
        let mut platform_registry = initialize_platform_probes(boot_info);
        initialize_storage_hardware(boot_info, &platform_registry);
        initialize_input_hardware(boot_info, &mut platform_registry);
        // Renoir platform probing is intentionally not executed from the early
        // x86_64 IDT/interrupt bring-up path. CPUID/platform detection is safe,
        // but the boot-phase/status plumbing can touch high-half state that is
        // not guaranteed mapped while exception vectors are being installed.
        // Re-enable from the supervisor/platform bring-up phase after IDT+PIC
        // are marked OK.
        // let _renoir_boot_profile = platform::amd::renoir_kernel_boot_probe(boot_info);
    }
}

#[cfg(all(not(feature = "emergency-boot"), feature = "hw-framebuffer"))]
fn initialize_framebuffer_console(boot_info: &BootInfo) {
    if boot_info.framebuffer.is_none() {
        boot_phase_skipped(
            BootPhase::Framebuffer,
            "framebuffer unavailable; serial console only",
        );
        crate::kprintln!("framebuffer unavailable; serial console only");
        return;
    }

    boot_phase_start(BootPhase::Framebuffer);
    match framebuffer_console::init_from_boot_info(boot_info) {
        Ok(Some(framebuffer)) => {
            boot_phase_online(BootPhase::Framebuffer);
            crate::kprintln!("Mirage framebuffer online");
            crate::kprintln!("  resolution: {}x{}", framebuffer.width, framebuffer.height);
            crate::kprintln!("  pitch: {}", framebuffer.pitch);
            crate::kprintln!("  bits per pixel: {}", framebuffer.bits_per_pixel);
            crate::kprintln!("  framebuffer address: {:#018x}", framebuffer.address.0);
        }
        Ok(None) | Err(_) => {
            boot_phase_failed(BootPhase::Framebuffer, "framebuffer initialization failed");
            crate::kprintln!("framebuffer initialization failed; serial console only");
        }
    }
}

#[cfg(all(not(feature = "emergency-boot"), not(feature = "hw-framebuffer")))]
fn initialize_framebuffer_console(_boot_info: &BootInfo) {
    boot_phase_skipped(BootPhase::Framebuffer, "framebuffer feature disabled");
    crate::kprintln!("framebuffer unavailable; serial console only");
}

/// Run a scheduled thread until hardware returns control through a trap.
///
/// The x86_64 path restores the thread's saved interrupt frame and returns to
/// the privilege level captured in [`CpuContext`](crate::kernel::thread::CpuContext).
/// Control comes back only after an interrupt or syscall entry stub saves a new
/// frame in the same context. Unit tests use the same register ABI by staging a
/// trap frame in the thread context before invoking the scheduler.
pub fn run_thread_slice(mut run_context: ThreadSliceRunContext<'_>) -> ThreadRunOutcome {
    let timer_epoch = idt::timer_ticks();

    #[cfg(not(test))]
    if run_context.context.privilege_mode == crate::kernel::thread::PrivilegeMode::User
        && run_context.context.sanitize_user_return_frame().is_none()
    {
        return ThreadRunOutcome::UserEntryInvalid;
    }

    enter_thread_slice(&mut run_context);

    match run_context.context.trap_vector {
        SYSCALL_TRAP_VECTOR => ThreadRunOutcome::Syscall(SyscallTrap {
            thread: run_context.thread,
            number: SyscallFrame::from_cpu_context(run_context.context).number,
            args: SyscallFrame::from_cpu_context(run_context.context).args,
        }),
        TIMER_INTERRUPT_VECTOR => {
            run_context.context.clear_trap();
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
#[cfg(not(test))]
pub fn switch_to_thread(mut run_context: ThreadSliceRunContext<'_>) {
    enter_thread_slice(&mut run_context);
}

/// Publish the current hardware scheduler identity and restore a thread frame.
///
/// Interrupt and syscall assembly reads these atomics when it builds a
/// [`CpuContext`] trap frame, then calls back into Rust to copy that frame into
/// the running [`ThreadControlBlock`].
pub fn enter_thread_slice(run_context: &mut ThreadSliceRunContext<'_>) {
    prepare_core_entry_state(run_context.core_index, run_context.kernel_stack_top);

    __mirage_current_core.store(run_context.core_index, Ordering::SeqCst);
    __mirage_current_thread.store(run_context.thread.raw(), Ordering::SeqCst);
    CURRENT_CONTEXT.store(
        run_context.context as *mut CpuContext as usize,
        Ordering::SeqCst,
    );

    #[cfg(not(test))]
    {
        if paging::switch_address_space(run_context.address_space_root).is_some() {
            unsafe {
                __mirage_context_restore(run_context.context as *mut CpuContext);
            }
        }
    }

    #[cfg(test)]
    {
        let _ = (
            run_context.process,
            run_context.address_space_root,
            run_context.kernel_stack_top,
        );
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
    prepare_core_entry_state(0, gdt::kernel_stack_top(0));
}

fn prepare_core_entry_state(core_index: usize, kernel_stack_top: u64) {
    let index = if core_index < MAX_CORES {
        core_index
    } else {
        0
    };
    let stack_top = if kernel_stack_top == 0 {
        gdt::kernel_stack_top(index)
    } else {
        kernel_stack_top
    };
    unsafe {
        PER_CPU[index].kernel_stack_top = stack_top;
    }
    gdt::set_tss_rsp0(stack_top);
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

/// Poll all registered early keyboard paths for the ESC debug-shell hotkey.
pub fn poll_debug_shell_hotkey() -> bool {
    #[cfg(feature = "hw-ps2-keyboard")]
    PS2_KEYBOARD_DRIVER.poll_hardware();
    #[cfg(feature = "hw-laptop-hotkeys")]
    ACPI_EC_HOTKEY_DRIVER.poll();
    crate::kernel::input::poll_debug_escape()
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

#[cfg(all(not(feature = "emergency-boot"), feature = "hw-usb-hid"))]
fn mark_driver_phase(phase: BootPhase, status: DriverStatus, skipped: &'static str) {
    match status {
        DriverStatus::Online => {
            boot_phase_start(phase);
            boot_phase_online(phase);
        }
        DriverStatus::Initialized => {
            boot_phase_start(phase);
            boot_phase_ok(phase);
        }
        DriverStatus::Skipped => boot_phase_skipped(phase, skipped),
        DriverStatus::Failed => boot_phase_failed(phase, "driver module failed"),
        DriverStatus::Registered => boot_phase_skipped(phase, "driver module did not start"),
    }
}

#[cfg(not(feature = "emergency-boot"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CpuProbe {
    vendor_ebx: u32,
    vendor_edx: u32,
    vendor_ecx: u32,
    family: u16,
    model: u16,
    stepping: u8,
    max_standard_leaf: u32,
    max_extended_leaf: u32,
    brand_string: [u8; 48],
    feature_ecx: u32,
    feature_edx: u32,
    extended_feature_ecx: u32,
    extended_feature_edx: u32,
    physical_address_bits: u8,
    virtual_address_bits: u8,
    apic_id: u32,
    logical_threads: u16,
    physical_cores: u16,
    threads_per_core: u16,
    package_id: u16,
    core_id: Option<u16>,
    xsave: bool,
    osxsave: bool,
    apic: bool,
    x2apic: bool,
    invariant_tsc: bool,
}

#[cfg(not(feature = "emergency-boot"))]
fn cpuid_count(leaf: u32, subleaf: u32) -> core::arch::x86_64::CpuidResult {
    unsafe { core::arch::x86_64::__cpuid_count(leaf, subleaf) }
}

#[cfg(not(feature = "emergency-boot"))]
fn probe_cpu() -> CpuProbe {
    crate::boot_trace_substep!("[ryzen 01]", "enter AMD64 CPU probe");
    crate::kprintln!("[ryzen 01] enter AMD64 CPU probe");
    let vendor = cpuid_count(0, 0);
    let max_standard_leaf = vendor.eax;
    let ext0 = cpuid_count(0x8000_0000, 0);
    let max_extended_leaf = ext0.eax;
    crate::boot_trace_substep!("[ryzen 02]", "CPUID max leaves read");
    crate::kprintln!("[ryzen 02] CPUID max leaves read");

    let features = if max_standard_leaf >= 1 {
        cpuid_count(1, 0)
    } else {
        cpuid_count(0, 0)
    };
    let family_id = ((features.eax >> 8) & 0x0f) as u16;
    let model_id = ((features.eax >> 4) & 0x0f) as u16;
    let ext_family = ((features.eax >> 20) & 0xff) as u16;
    let ext_model = ((features.eax >> 16) & 0x0f) as u16;
    let family = if family_id == 0x0f {
        family_id.saturating_add(ext_family)
    } else {
        family_id
    };
    let model = if family_id == 0x06 || family_id == 0x0f {
        (ext_model << 4) | model_id
    } else {
        model_id
    };
    let apic_id = if max_standard_leaf >= 1 {
        (features.ebx >> 24) & 0xff
    } else {
        0
    };
    let logical_threads = if max_standard_leaf >= 1 {
        ((features.ebx >> 16) & 0xff).max(1) as u16
    } else {
        1
    };
    crate::boot_trace_substep!("[ryzen 03]", "vendor/family/model parsed");
    crate::kprintln!("[ryzen 03] vendor/family/model parsed");

    let mut brand_string = [0u8; 48];
    if max_extended_leaf >= 0x8000_0004 {
        let leaves = [
            cpuid_count(0x8000_0002, 0),
            cpuid_count(0x8000_0003, 0),
            cpuid_count(0x8000_0004, 0),
        ];
        let mut out = 0usize;
        let mut index = 0usize;
        while index < leaves.len() {
            let leaf = leaves[index];
            for reg in [leaf.eax, leaf.ebx, leaf.ecx, leaf.edx] {
                brand_string[out..out + 4].copy_from_slice(&reg.to_le_bytes());
                out += 4;
            }
            index += 1;
        }
        crate::boot_trace_substep!("[ryzen 04]", "brand string parsed or skipped");
        crate::kprintln!("[ryzen 04] brand string parsed or skipped");
    } else {
        crate::boot_trace_substep!("[ryzen 04]", "brand string parsed or skipped");
        crate::kprintln!("[ryzen 04] brand string parsed or skipped");
    }

    crate::boot_trace_substep!("[ryzen 05]", "topology probe enter");
    crate::kprintln!("[ryzen 05] topology probe enter");
    let ext1 = if max_extended_leaf >= 0x8000_0001 {
        cpuid_count(0x8000_0001, 0)
    } else {
        cpuid_count(0, 0)
    };
    let ext7 = if max_extended_leaf >= 0x8000_0007 {
        cpuid_count(0x8000_0007, 0)
    } else {
        cpuid_count(0, 0)
    };
    let ext8 = if max_extended_leaf >= 0x8000_0008 {
        cpuid_count(0x8000_0008, 0)
    } else {
        cpuid_count(0, 0)
    };
    let ext1e_available = max_extended_leaf >= 0x8000_001e;
    let ext1e = if ext1e_available {
        cpuid_count(0x8000_001e, 0)
    } else {
        cpuid_count(0, 0)
    };
    let physical_cores = if max_extended_leaf >= 0x8000_0008 {
        ((ext8.ecx & 0xff) as u16).saturating_add(1).max(1)
    } else {
        logical_threads.max(1)
    };
    let threads_per_core = if ext1e_available && ext1e.ebx != 0 {
        (((ext1e.ebx >> 8) & 0xff) as u16).saturating_add(1).max(1)
    } else {
        (logical_threads / physical_cores.max(1)).max(1)
    };
    let package_id = if ext1e_available {
        (ext1e.ecx & 0xff) as u16
    } else {
        (apic_id as u16) / logical_threads.max(1)
    };
    let core_id = if ext1e_available {
        Some((ext1e.ebx & 0xff) as u16)
    } else {
        None
    };
    crate::boot_trace_substep!("[ryzen 06]", "topology parsed or skipped");
    crate::kprintln!("[ryzen 06] topology parsed or skipped");
    crate::boot_trace_substep!("[ryzen 07]", "MSR telemetry skipped or safe");
    crate::kprintln!("[ryzen 07] MSR telemetry skipped or safe");

    CpuProbe {
        vendor_ebx: vendor.ebx,
        vendor_edx: vendor.edx,
        vendor_ecx: vendor.ecx,
        family,
        model,
        stepping: (features.eax & 0x0f) as u8,
        max_standard_leaf,
        max_extended_leaf,
        brand_string,
        feature_ecx: features.ecx,
        feature_edx: features.edx,
        extended_feature_ecx: ext1.ecx,
        extended_feature_edx: ext1.edx,
        physical_address_bits: if max_extended_leaf >= 0x8000_0008 {
            (ext8.eax & 0xff) as u8
        } else {
            0
        },
        virtual_address_bits: if max_extended_leaf >= 0x8000_0008 {
            ((ext8.eax >> 8) & 0xff) as u8
        } else {
            0
        },
        apic_id,
        logical_threads,
        physical_cores,
        threads_per_core,
        package_id,
        core_id,
        xsave: (features.ecx & (1 << 26)) != 0,
        osxsave: (features.ecx & (1 << 27)) != 0,
        apic: (features.edx & (1 << 9)) != 0,
        x2apic: (features.ecx & (1 << 21)) != 0,
        invariant_tsc: (ext7.edx & (1 << 8)) != 0,
    }
}

#[cfg(not(feature = "emergency-boot"))]
fn cpu_vendor_is_amd(cpu: CpuProbe) -> bool {
    cpu.vendor_ebx == 0x6874_7541 && cpu.vendor_edx == 0x6974_6e65 && cpu.vendor_ecx == 0x444d_4163
}

#[cfg(not(feature = "emergency-boot"))]
fn cpu_is_supported_ryzen(cpu: CpuProbe) -> bool {
    cpu_vendor_is_amd(cpu) && matches!(cpu.family, 0x17 | 0x19)
}

#[cfg(not(feature = "emergency-boot"))]
fn cpu_topology_is_complete(cpu: CpuProbe) -> bool {
    cpu.logical_threads >= 1 && cpu.physical_cores >= 1 && cpu.threads_per_core >= 1
}

#[cfg(not(feature = "emergency-boot"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PciProbeDevice {
    bus: u8,
    device: u8,
    function: u8,
    vendor_id: u16,
    device_id: u16,
    revision: u8,
    class: u8,
    subclass: u8,
    prog_if: u8,
    header_type: u8,
    bars: [Option<PlatformPciBar>; 6],
    irq_line: u8,
}

impl PciProbeDevice {
    const fn platform_name(self) -> &'static str {
        if let Some(device) = mirage_device_db::lookup_pci_device(self.vendor_id, self.device_id) {
            device.name
        } else if let Some(class) =
            mirage_device_db::lookup_pci_class(self.class, self.subclass, self.prog_if)
        {
            class.name
        } else if let Some(vendor) = mirage_device_db::lookup_pci_vendor(self.vendor_id) {
            vendor.name
        } else {
            "Unknown PCI Device"
        }
    }

    const fn platform_kind(self) -> mirage_platform::PlatformDeviceKind {
        if self.class == 0x01 {
            mirage_platform::PlatformDeviceKind::Storage
        } else if self.class == 0x03 {
            mirage_platform::PlatformDeviceKind::Display
        } else if self.class == 0x0c && self.subclass == 0x03 {
            mirage_platform::PlatformDeviceKind::Usb
        } else {
            mirage_platform::PlatformDeviceKind::Pci
        }
    }

    const fn platform_device(self) -> mirage_platform::PlatformDevice {
        mirage_platform::PlatformDevice::pci(
            self.platform_name(),
            self.platform_kind(),
            self.bus,
            self.device,
            self.function,
            self.vendor_id,
            self.device_id,
            self.class,
            self.subclass,
            self.prog_if,
            self.header_type,
        )
        .with_bars(self.bars)
        .with_irq_line(self.irq_line)
    }
}

#[cfg(not(feature = "emergency-boot"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PciProbeFunction {
    bus: u8,
    device: u8,
    function: u8,
}

#[cfg(not(feature = "emergency-boot"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PciClassFields {
    revision: u8,
    prog_if: u8,
    subclass: u8,
    class_code: u8,
}

#[cfg(not(feature = "emergency-boot"))]
const PCI_CONFIG_ADDRESS_PORT: u16 = 0x0cf8;
#[cfg(not(feature = "emergency-boot"))]
const PCI_CONFIG_DATA_PORT: u16 = 0x0cfc;

#[cfg(not(feature = "emergency-boot"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PciConfigBackend {
    /// Legacy PCI mechanism #1. This is the only backend used until ACPI MCFG
    /// is parsed and the ECAM/MMCONFIG physical window is mapped by VM code.
    LegacyCf8Cfc,
}

#[cfg(not(feature = "emergency-boot"))]
const fn pci_config_backend() -> PciConfigBackend {
    PciConfigBackend::LegacyCf8Cfc
}

#[cfg(not(feature = "emergency-boot"))]
const fn pci_config_address(function: PciProbeFunction, offset: u8) -> u32 {
    0x8000_0000u32
        | ((function.bus as u32) << 16)
        | ((function.device as u32) << 11)
        | ((function.function as u32) << 8)
        | ((offset as u32) & 0xfc)
}

#[cfg(not(feature = "emergency-boot"))]
const fn pci_vendor_id(raw_id: u32) -> u16 {
    (raw_id & 0xffff) as u16
}

#[cfg(not(feature = "emergency-boot"))]
const fn pci_device_id(raw_id: u32) -> u16 {
    (raw_id >> 16) as u16
}

#[cfg(not(feature = "emergency-boot"))]
const fn pci_device_present(raw_id: u32) -> bool {
    pci_vendor_id(raw_id) != 0xffff
}

#[cfg(not(feature = "emergency-boot"))]
const fn pci_class_fields(class_reg: u32) -> PciClassFields {
    PciClassFields {
        revision: (class_reg & 0xff) as u8,
        prog_if: ((class_reg >> 8) & 0xff) as u8,
        subclass: ((class_reg >> 16) & 0xff) as u8,
        class_code: ((class_reg >> 24) & 0xff) as u8,
    }
}

#[cfg(not(feature = "emergency-boot"))]
const fn pci_header_type(header_reg: u32) -> u8 {
    ((header_reg >> 16) & 0xff) as u8
}

#[cfg(not(feature = "emergency-boot"))]
fn pci_probe_bars(function: PciProbeFunction, header_type: u8) -> [Option<PlatformPciBar>; 6] {
    let mut bars = [None; 6];
    if (header_type & 0x7f) != 0x00 {
        return bars;
    }
    let mut index = 0usize;
    while index < 6 {
        let offset = 0x10 + (index as u8 * 4);
        let raw = pci_probe_read_u32(function, offset);
        if raw == 0 {
            index += 1;
            continue;
        }
        if raw & 0x1 != 0 {
            bars[index] = Some(PlatformPciBar::io(index as u8, raw));
            index += 1;
        } else {
            let bar_type = (raw >> 1) & 0x3;
            if bar_type == 0x2 && index + 1 < 6 {
                let high = pci_probe_read_u32(function, offset + 4);
                bars[index] = Some(PlatformPciBar::mmio64(index as u8, raw, high));
                index += 2;
            } else {
                bars[index] = Some(PlatformPciBar::mmio32(index as u8, raw));
                index += 1;
            }
        }
    }
    bars
}

#[cfg(not(feature = "emergency-boot"))]
fn pci_interrupt_line(function: PciProbeFunction, header_type: u8) -> u8 {
    if (header_type & 0x7f) == 0x00 {
        (pci_probe_read_u32(function, 0x3c) & 0xff) as u8
    } else {
        0xff
    }
}

#[cfg(not(feature = "emergency-boot"))]
const fn pci_is_multifunction(header_type: u8) -> bool {
    (header_type & 0x80) != 0
}

#[cfg(not(feature = "emergency-boot"))]
const fn pci_vendor_name(vendor_id: u16) -> &'static str {
    match vendor_id {
        0x1022 => "AMD",
        0x1002 => "AMD/ATI",
        0x8086 => "Intel",
        0x10ec => "Realtek",
        0x1b36 => "Red Hat/QEMU",
        0x1234 => "QEMU",
        0x1af4 => "VirtIO",
        0x144d => "Samsung",
        0x15ad => "VMware",
        _ => "unknown vendor",
    }
}

#[cfg(not(feature = "emergency-boot"))]
const fn pci_class_name(class_code: u8) -> &'static str {
    match class_code {
        0x01 => "storage",
        0x02 => "network",
        0x03 => "display",
        0x04 => "multimedia",
        0x06 => "bridge",
        0x0c => "serial bus",
        0x0d => "wireless",
        _ => "unknown class",
    }
}

#[cfg(not(feature = "emergency-boot"))]
fn pci_probe_read_u32(function: PciProbeFunction, offset: u8) -> u32 {
    let address = pci_config_address(function, offset);
    unsafe {
        crate::arch::x86_64::io::outl(PCI_CONFIG_ADDRESS_PORT, address);
        crate::arch::x86_64::io::inl(PCI_CONFIG_DATA_PORT)
    }
}

#[cfg(not(feature = "emergency-boot"))]
fn pci_probe_function(function: PciProbeFunction) -> Option<PciProbeDevice> {
    let raw_id = pci_probe_read_u32(function, 0x00);
    let vendor_id = pci_vendor_id(raw_id);
    let device_id = pci_device_id(raw_id);

    if ryzen_debug_pci() && function.bus == 0 && function.device < 4 {
        crate::kprintln!(
            "[pci] {:02x}:{:02x}.{} raw id=0x{:08x} vendor=0x{:04x} device=0x{:04x}",
            function.bus,
            function.device,
            function.function,
            raw_id,
            vendor_id,
            device_id
        );
    }

    if !pci_device_present(raw_id) {
        return None;
    }

    let class_reg = pci_probe_read_u32(function, 0x08);
    let header_reg = pci_probe_read_u32(function, 0x0c);
    let class = pci_class_fields(class_reg);
    let header_type = pci_header_type(header_reg);

    if ryzen_debug_pci() && function.bus == 0 && function.device < 4 {
        crate::kprintln!(
            "[pci] {:02x}:{:02x}.{} class=0x{:02x} subclass=0x{:02x} prog_if=0x{:02x} header=0x{:02x} vendor_name=\"{}\" class_name=\"{}\"",
            function.bus,
            function.device,
            function.function,
            class.class_code,
            class.subclass,
            class.prog_if,
            header_type,
            pci_vendor_name(vendor_id),
            pci_class_name(class.class_code)
        );
    }

    Some(PciProbeDevice {
        bus: function.bus,
        device: function.device,
        function: function.function,
        vendor_id,
        device_id,
        revision: class.revision,
        class: class.class_code,
        subclass: class.subclass,
        prog_if: class.prog_if,
        header_type,
        bars: pci_probe_bars(function, header_type),
        irq_line: pci_interrupt_line(function, header_type),
    })
}

#[cfg(not(feature = "emergency-boot"))]
fn scan_pci_devices(mut visitor: impl FnMut(PciProbeDevice)) {
    crate::boot_trace_substep!("[renoir 02]", "PCI scan start");
    match pci_config_backend() {
        PciConfigBackend::LegacyCf8Cfc => {
            if ryzen_debug_pci() {
                crate::kprintln!("[pci] config access: legacy CF8/CFC");
            }
        }
    }

    // Only bus 0 is trusted until bridge enumeration is implemented.
    let bus = 0u8;
    let mut device = 0u8;
    while device < 32 {
        let function0 = PciProbeFunction {
            bus,
            device,
            function: 0,
        };
        if let Some(device0) = pci_probe_function(function0) {
            crate::boot_trace_substep!("[renoir 03]", "PCI device read");
            visitor(device0);
            if pci_is_multifunction(device0.header_type) {
                let mut function = 1u8;
                while function < 8 {
                    if let Some(found) = pci_probe_function(PciProbeFunction {
                        bus,
                        device,
                        function,
                    }) {
                        crate::boot_trace_substep!("[renoir 03]", "PCI device read");
                        visitor(found);
                    }
                    function += 1;
                }
            }
        }
        device += 1;
    }
}

#[cfg(not(feature = "emergency-boot"))]
const fn ryzen_debug_pci() -> bool {
    crate::kernel::boot_diagnostics::debug_pci_enabled()
        || crate::kernel::boot_diagnostics::raw_hw_dump_enabled()
}

#[cfg(not(feature = "emergency-boot"))]
fn initialize_platform_probes(
    boot_info: &BootInfo,
) -> mirage_platform::PlatformRegistry<{ mirage_platform::MAX_PLATFORM_DEVICE_EVENTS }> {
    let cpu = probe_cpu();
    let mut registry = mirage_platform::PlatformRegistry::new();
    crate::kernel::platform::register_platform_device(
        &mut registry,
        mirage_platform::PlatformDevice::amd_cpu(
            cpu_platform_name(cpu),
            cpu.family.min(u8::MAX as u16) as u8,
            cpu.model.min(u8::MAX as u16) as u8,
            cpu.stepping,
        ),
    );

    scan_pci_devices(|device| {
        crate::kernel::platform::register_platform_device(&mut registry, device.platform_device());
    });

    if boot_info.rsdp.is_some() {
        crate::kernel::platform::register_platform_device(
            &mut registry,
            mirage_platform::PlatformDevice::acpi_table("RSDP"),
        );
    }

    boot_phase_start(BootPhase::Amd64Cpu);
    if cpu_vendor_is_amd(cpu) {
        boot_phase_ok(BootPhase::Amd64Cpu);
    } else {
        boot_phase_skipped(BootPhase::Amd64Cpu, "CPUID vendor is not AuthenticAMD");
    }

    boot_phase_start(BootPhase::RyzenCpu);
    if cpu_is_supported_ryzen(cpu) {
        boot_phase_ok(BootPhase::RyzenCpu);
    } else {
        boot_phase_skipped(
            BootPhase::RyzenCpu,
            "supported Ryzen/Renoir CPU not detected",
        );
    }

    boot_phase_start(BootPhase::RyzenTopology);
    if cpu_is_supported_ryzen(cpu) && cpu_topology_is_complete(cpu) {
        boot_phase_ok(BootPhase::RyzenTopology);
    } else if cpu_is_supported_ryzen(cpu) && cpu.logical_threads >= 1 {
        boot_phase_detected(BootPhase::RyzenTopology);
    } else {
        boot_phase_skipped(BootPhase::RyzenTopology, "CPUID topology unavailable");
    }
    crate::boot_trace_substep!("[ryzen 08]", "platform facts committed");
    crate::kprintln!("[ryzen 08] platform facts committed");

    crate::boot_trace_substep!("[renoir 01]", "enter platform inventory");
    crate::boot_trace_substep!("[renoir 02]", "PCI scan start");
    crate::kprintln!("[renoir 01] enter platform inventory");
    let amd_soc = registry.find_amd_soc_device().is_some();
    boot_phase_start(BootPhase::AmdSoc);
    if amd_soc {
        boot_phase_detected(BootPhase::AmdSoc);
    } else {
        boot_phase_skipped(BootPhase::AmdSoc, "AMD SoC PCI devices not present");
    }
    crate::boot_trace_substep!("[renoir 04]", "AMD device classified");
    crate::kprintln!("[renoir 04] AMD device classified");

    boot_phase_start(BootPhase::AmdIommu);
    if boot_info.rsdp.is_some() && amd_soc {
        boot_phase_stub(BootPhase::AmdIommu, "IVRS parser not implemented");
    } else {
        boot_phase_skipped(BootPhase::AmdIommu, "AMD IVRS/IOMMU not detected");
    }

    boot_phase_start(BootPhase::AcpiTables);
    if boot_info.rsdp.is_some() {
        boot_phase_ok(BootPhase::AcpiTables);
    } else {
        boot_phase_skipped(BootPhase::AcpiTables, "RSDP not provided by bootloader");
    }

    boot_phase_start(BootPhase::Thermal);
    boot_phase_skipped(BootPhase::Thermal, "thermal ACPI probe not implemented");

    boot_phase_start(BootPhase::Battery);
    boot_phase_skipped(BootPhase::Battery, "battery ACPI probe not implemented");

    crate::boot_trace_substep!("[renoir 05]", "AMDGPU detection skipped/detected");
    let renoir_gpu = registry.find_amdgpu_renoir().is_some();
    boot_phase_start(BootPhase::AmdGpuRenoir);
    if renoir_gpu {
        boot_phase_detected(BootPhase::AmdGpuRenoir);
    } else {
        boot_phase_skipped(BootPhase::AmdGpuRenoir, "Renoir GPU PCI device not present");
    }

    crate::boot_trace_substep!("[renoir 06]", "AMD xHCI detection start");
    let amd_xhci = registry
        .find_xhci()
        .is_some_and(|device| device.vendor_id == Some(0x1022));
    boot_phase_start(BootPhase::AmdXhci);
    if amd_xhci {
        boot_phase_detected(BootPhase::AmdXhci);
    } else {
        boot_phase_skipped(BootPhase::AmdXhci, "AMD xHCI controller not present");
    }
    crate::boot_trace_substep!("[renoir 07]", "AMD xHCI detection done");
    crate::boot_trace_substep!("[renoir 08]", "exit platform inventory");
    crate::kprintln!("[renoir 08] exit platform inventory");

    registry
}

#[cfg(not(feature = "emergency-boot"))]
fn cpu_platform_name(cpu: CpuProbe) -> &'static str {
    if cpu_vendor_is_amd(cpu) && cpu.family == 0x17 && cpu.model == 0x60 {
        "AMD Ryzen 5 4500U"
    } else if cpu_vendor_is_amd(cpu) && matches!(cpu.family, 0x17 | 0x19) {
        "AMD Ryzen CPU"
    } else if cpu_vendor_is_amd(cpu) {
        "AMD64 CPU"
    } else {
        "x86_64 CPU"
    }
}

#[cfg(not(feature = "emergency-boot"))]
#[allow(unused_mut, unused_variables)]
fn initialize_storage_hardware(
    boot_info: &BootInfo,
    platform: &mirage_platform::PlatformRegistry<{ mirage_platform::MAX_PLATFORM_DEVICE_EVENTS }>,
) {
    boot_phase_start(BootPhase::BlockLayer);
    boot_phase_ok(BootPhase::BlockLayer);

    let nvme_present = platform.platform_find_nvme_controller().is_some();
    let ahci_present = platform.platform_find_ahci_controller().is_some();

    if nvme_present {
        boot_phase_detected(BootPhase::Nvme);
        boot_phase_start(BootPhase::Nvme);
        // Kernel policy is honest: discovering PCIe NVMe hardware is not the same as
        // registering a namespace. The hardware driver crates contain bounded queue
        // mechanics, but this early arch path does not claim Online until a namespace
        // is registered through the block layer.
        boot_phase_failed(
            BootPhase::Nvme,
            "NVMe controller detected; namespace registration not wired in this boot path",
        );
        boot_phase_skipped(BootPhase::NvmeNamespace, "no NVMe namespace registered");
    } else {
        boot_phase_skipped(BootPhase::Nvme, "NVMe controller not present");
        boot_phase_skipped(BootPhase::NvmeNamespace, "NVMe controller not present");
    }

    let mut sata_online = false;
    if ahci_present {
        boot_phase_detected(BootPhase::Ahci);
        boot_phase_start(BootPhase::Ahci);
        #[cfg(feature = "hw-ahci")]
        {
            match ahci::bring_up_first_sata_disk(platform, boot_info.hhdm_offset) {
                ahci::AhciBootStatus::Online(scan) => {
                    if let Some(info) = scan.sata_disk {
                        sata_online = true;
                        boot_phase_detected(BootPhase::SataDisk);
                        boot_phase_start(BootPhase::SataDisk);
                        boot_phase_online(BootPhase::SataDisk);
                        crate::kprintln!(
                            "[block] registered {} kind=SataDisk block_size={} blocks={}",
                            info.name,
                            info.block_size,
                            info.block_count
                        );
                    } else {
                        boot_phase_skipped(BootPhase::SataDisk, "no SATA disk detected");
                    }
                    if let Some(info) = scan.atapi_media {
                        boot_phase_detected(BootPhase::Atapi);
                        boot_phase_start(BootPhase::Atapi);
                        boot_phase_online(BootPhase::Atapi);
                        boot_phase_detected(BootPhase::OpticalDisk);
                        boot_phase_start(BootPhase::OpticalDisk);
                        boot_phase_online(BootPhase::OpticalDisk);
                        crate::kprintln!(
                            "[block] registered {} kind=OpticalDisk block_size={} blocks={}",
                            info.name,
                            info.block_size,
                            info.block_count
                        );
                    } else if scan.atapi_detected {
                        boot_phase_detected(BootPhase::Atapi);
                        boot_phase_skipped(
                            BootPhase::Atapi,
                            scan.atapi_probe_error.unwrap_or("ATAPI media probe failed"),
                        );
                        boot_phase_skipped(
                            BootPhase::OpticalDisk,
                            scan.atapi_probe_error.unwrap_or("ATAPI media probe failed"),
                        );
                    } else {
                        boot_phase_skipped(BootPhase::Atapi, "no ATAPI device detected");
                        boot_phase_skipped(
                            BootPhase::OpticalDisk,
                            "no ATAPI optical device detected",
                        );
                    }
                    boot_phase_online(BootPhase::Ahci);
                }
                ahci::AhciBootStatus::NoDisk(scan) => {
                    boot_phase_ok(BootPhase::Ahci);
                    boot_phase_skipped(BootPhase::SataDisk, "no SATA disk detected");
                    if scan.atapi_detected {
                        boot_phase_detected(BootPhase::Atapi);
                        boot_phase_skipped(
                            BootPhase::Atapi,
                            scan.atapi_probe_error.unwrap_or("ATAPI media probe failed"),
                        );
                        boot_phase_skipped(
                            BootPhase::OpticalDisk,
                            scan.atapi_probe_error.unwrap_or("ATAPI media probe failed"),
                        );
                    } else {
                        boot_phase_skipped(BootPhase::Atapi, "no ATAPI device detected");
                        boot_phase_skipped(
                            BootPhase::OpticalDisk,
                            "no ATAPI optical device detected",
                        );
                    }
                }
                ahci::AhciBootStatus::Failed(reason) => {
                    boot_phase_failed(BootPhase::Ahci, reason);
                    boot_phase_skipped(BootPhase::SataDisk, "AHCI initialization failed");
                }
            }
        }
        #[cfg(not(feature = "hw-ahci"))]
        {
            boot_phase_skipped(BootPhase::Ahci, "hw-ahci feature disabled");
            boot_phase_skipped(BootPhase::SataDisk, "hw-ahci feature disabled");
            boot_phase_skipped(BootPhase::Atapi, "hw-ahci feature disabled");
            boot_phase_skipped(BootPhase::OpticalDisk, "hw-ahci feature disabled");
        }
    } else {
        boot_phase_skipped(BootPhase::Ahci, "AHCI controller not present");
        boot_phase_skipped(BootPhase::SataDisk, "AHCI controller not present");
        boot_phase_skipped(BootPhase::Atapi, "AHCI controller not present");
        boot_phase_skipped(BootPhase::OpticalDisk, "AHCI controller not present");
    }

    if sata_online {
        boot_phase_online(BootPhase::M2Storage);
    } else if nvme_present || ahci_present {
        boot_phase_skipped(
            BootPhase::M2Storage,
            "no block device online for M.2-capable path",
        );
    } else {
        boot_phase_skipped(BootPhase::M2Storage, "no M.2-capable storage path present");
    }

    boot_phase_skipped(
        BootPhase::Qfs,
        "root QFS block mount deferred until block device selection",
    );
}

#[allow(unused_mut, unused_variables)]
fn initialize_input_hardware(
    boot_info: &BootInfo,
    platform: &mut mirage_platform::PlatformRegistry<
        { mirage_platform::MAX_PLATFORM_DEVICE_EVENTS },
    >,
) {
    let mut any_online = false;

    #[cfg(feature = "hw-i8042")]
    {
        crate::kernel::platform::register_platform_device(
            platform,
            mirage_platform::PlatformDevice::i8042(),
        );
        boot_phase_start(BootPhase::I8042);
        boot_phase_ok(BootPhase::I8042);
    }
    #[cfg(not(feature = "hw-i8042"))]
    boot_phase_skipped(BootPhase::I8042, "hw-i8042 feature disabled");

    #[cfg(feature = "hw-ps2-keyboard")]
    {
        boot_phase_start(BootPhase::Ps2Keyboard);
        // Keep early PS/2 input non-blocking during architecture bring-up.
        // IRQ1 delivery is not a boot dependency and enabling it here can leave
        // QEMU servicing keyboard interrupts before the post-kernel pipeline
        // reaches BootInfoApplied/Supervisor/MTSS.  Polling still allows the
        // debug-shell hotkey path to drain scancodes without gating boot on the
        // first key event.
        match PS2_KEYBOARD_DRIVER.initialize(false) {
            Ok(()) => {
                boot_phase_ok(BootPhase::Ps2Keyboard);
            }
            Err(error) => {
                boot_phase_failed(
                    BootPhase::Ps2Keyboard,
                    "PS/2 keyboard initialization failed",
                );
                crate::kprintln!("PS/2 keyboard initialization failed: {:?}", error);
            }
        }
    }
    #[cfg(not(feature = "hw-ps2-keyboard"))]
    boot_phase_skipped(BootPhase::Ps2Keyboard, "hw-ps2-keyboard feature disabled");

    #[cfg(feature = "hw-usb-hid")]
    {
        let usb_status =
            USB_HID_KEYBOARD_DRIVER.initialize_stack_with_platform(boot_info.hhdm_offset, platform);
        mark_driver_phase(
            BootPhase::Xhci,
            usb_status.xhci,
            "xHCI controller not present",
        );
        mark_driver_phase(
            BootPhase::UsbCore,
            usb_status.core,
            "usb-core0 dependency skipped",
        );
        mark_driver_phase(
            BootPhase::UsbHid,
            usb_status.hid,
            "USB HID device not present",
        );
        match usb_status.keyboard {
            XhciKeyboardStatus::Online => {
                boot_phase_start(BootPhase::UsbKeyboard);
                boot_phase_online(BootPhase::UsbKeyboard);
                any_online = true;
            }
            XhciKeyboardStatus::SkippedNoController => {
                boot_phase_skipped(BootPhase::UsbKeyboard, "xHCI controller not present")
            }
            XhciKeyboardStatus::SkippedNoKeyboard => {
                boot_phase_skipped(BootPhase::UsbKeyboard, "USB HID keyboard not present")
            }
            XhciKeyboardStatus::Failed(message) => {
                boot_phase_failed(BootPhase::UsbKeyboard, message);
                crate::kprintln!("USB HID keyboard initialization failed: {}", message);
            }
        }
        if usb_status.keyboard == XhciKeyboardStatus::Online {
            any_online = true;
        }
    }
    #[cfg(not(feature = "hw-usb-hid"))]
    {
        boot_phase_skipped(BootPhase::Xhci, "hw-usb-hid feature disabled");
        boot_phase_skipped(BootPhase::UsbCore, "hw-usb-hid feature disabled");
        boot_phase_skipped(BootPhase::UsbHid, "hw-usb-hid feature disabled");
        boot_phase_skipped(BootPhase::UsbKeyboard, "hw-usb-hid feature disabled");
    }

    #[cfg(feature = "hw-acpi-ec")]
    {
        boot_phase_start(BootPhase::AcpiEc);
        boot_phase_ok(BootPhase::AcpiEc);
    }
    #[cfg(not(feature = "hw-acpi-ec"))]
    boot_phase_skipped(BootPhase::AcpiEc, "hw-acpi-ec feature disabled");

    #[cfg(feature = "hw-laptop-hotkeys")]
    {
        boot_phase_start(BootPhase::EcHotkeys);
        match ACPI_EC_HOTKEY_DRIVER.initialize(boot_info) {
            AcpiEcStatus::Online => {
                boot_phase_ok(BootPhase::EcHotkeys);
                any_online = true;
            }
            AcpiEcStatus::SkippedNoAcpi => boot_phase_skipped(BootPhase::EcHotkeys, "ACPI absent"),
            AcpiEcStatus::SkippedNoEc => boot_phase_skipped(BootPhase::EcHotkeys, "EC absent"),
        }
    }
    #[cfg(not(feature = "hw-laptop-hotkeys"))]
    boot_phase_skipped(BootPhase::EcHotkeys, "hw-laptop-hotkeys feature disabled");

    boot_phase_start(BootPhase::Input);
    if any_online || crate::kernel::input::any_keyboard_online() {
        boot_phase_ok(BootPhase::Input);
    } else {
        boot_phase_skipped(BootPhase::Input, "no keyboard source online");
    }
}

#[cfg(not(feature = "emergency-boot"))]
fn configure_cpu_modes() {
    interrupts::disable();
    boot_phase_start(BootPhase::Gdt);
    gdt::initialize();
    boot_phase_ok(BootPhase::Gdt);
    crate::kprintln!("GDT initialized");
}

#[cfg(not(feature = "emergency-boot"))]
fn setup_memory_layout(boot_info: &BootInfo) {
    boot_phase_start(BootPhase::MemoryMap);
    validate_boot_memory_handoff(boot_info);
    boot_phase_ok(BootPhase::MemoryMap);

    boot_phase_start(BootPhase::KernelMapper);
    paging::initialize(boot_info);
    boot_phase_ok(BootPhase::KernelMapper);
    boot_phase_start(BootPhase::Paging);
    boot_phase_ok(BootPhase::Paging);

    boot_phase_start(BootPhase::PhysicalAllocator);
    memory::initialize_from_boot_info(boot_info);
    boot_phase_ok(BootPhase::PhysicalAllocator);
    boot_phase_start(BootPhase::Heap);
    boot_phase_online(BootPhase::Heap);
    boot_phase_start(BootPhase::Memory);
    boot_phase_ok(BootPhase::Memory);
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
#[cfg(not(feature = "emergency-boot"))]
fn configure_interrupts(boot_info: &BootInfo) {
    boot_phase_start(BootPhase::Idt);
    idt::initialize();
    boot_phase_ok(BootPhase::Idt);

    if let Some(madt) = {
        // TEMP: ACPI MADT discovery is disabled during early IDT/APIC bring-up.
        // The current page tables do not map the firmware/ACPI physical memory
        // window yet, so acpi_madt::discover(boot_info) can fault on high-half
        // direct-map reads before the platform-memory phase exists.
        //
        // Re-enable this from a later ACPI_EARLY/MADT phase after the ACPI
        // table pages are explicitly mapped.
        let _ = boot_info;
        None::<acpi_madt::MadtInfo>
    } {
        boot_phase_start(BootPhase::AcpiTables);
        boot_phase_ok(BootPhase::AcpiTables);
        if let Some(hhdm) = boot_info.hhdm_offset {
            pic::mask_all();
            match apic::initialize(madt.local_apic_address, hhdm) {
                apic::LocalApicStatus::Enabled {
                    physical_base,
                    apic_id,
                } => crate::kprintln!(
                    "x86_64: local APIC enabled phys={:#x} apic_id={}",
                    physical_base,
                    apic_id
                ),
                apic::LocalApicStatus::Unavailable => {
                    boot_phase_failed(BootPhase::Interrupts, "local APIC unavailable")
                }
            }
            match ioapic::initialize_from_madt(&madt, hhdm) {
                ioapic::IoApicStatus::Enabled {
                    physical_base,
                    gsi_base,
                    max_redirection_entries,
                } => {
                    irq::select_apic();
                    boot_phase_skipped(BootPhase::Pic, "legacy PIC masked; APIC/IOAPIC active");
                    crate::kprintln!(
                        "x86_64: IOAPIC enabled phys={:#x} gsi_base={} entries={}",
                        physical_base,
                        gsi_base,
                        max_redirection_entries
                    );
                }
                ioapic::IoApicStatus::Unavailable => {
                    irq::select_pic();
                    pic::initialize();
                    boot_phase_ok(BootPhase::Pic);
                    crate::kprintln!(
                        "x86_64: MADT present but IOAPIC unavailable; using legacy PIC"
                    );
                }
            }
        } else {
            irq::select_pic();
            pic::initialize();
            boot_phase_ok(BootPhase::Pic);
            crate::kprintln!("x86_64: missing HHDM; using legacy PIC interrupt path");
        }
    } else {
        boot_phase_skipped(BootPhase::AcpiTables, "MADT unavailable; using legacy PIC");
        irq::select_pic();
        pic::initialize();
        boot_phase_ok(BootPhase::Pic);
    }
    interrupts::enable();
    boot_phase_online(BootPhase::Interrupts);
    crate::kprintln!("interrupts enabled");
}

#[cfg(all(test, not(feature = "emergency-boot")))]
mod pci_config_tests {
    use super::*;

    #[test]
    fn pci_config_address_uses_legacy_cf8_formula() {
        let function = PciProbeFunction {
            bus: 0x12,
            device: 0x05,
            function: 0x03,
        };

        assert_eq!(
            pci_config_address(function, 0x13),
            0x8000_0000 | (0x12 << 16) | (0x05 << 11) | (0x03 << 8) | 0x10
        );
    }

    #[test]
    fn extracts_vendor_and_device_from_raw_id() {
        let raw = 0x5678_1234;

        assert_eq!(pci_vendor_id(raw), 0x1234);
        assert_eq!(pci_device_id(raw), 0x5678);
    }

    #[test]
    fn detects_absent_vendor_id() {
        assert!(!pci_device_present(0xffff_ffff));
        assert!(!pci_device_present(0x0001_ffff));
        assert!(pci_device_present(0x0001_8086));
    }

    #[test]
    fn extracts_revision_class_subclass_and_prog_if() {
        let fields = pci_class_fields(0x0106_027a);

        assert_eq!(fields.revision, 0x7a);
        assert_eq!(fields.prog_if, 0x02);
        assert_eq!(fields.subclass, 0x06);
        assert_eq!(fields.class_code, 0x01);
    }

    #[test]
    fn extracts_header_type_and_multifunction_bit() {
        assert_eq!(pci_header_type(0x0080_0000), 0x80);
        assert!(pci_is_multifunction(0x80));
        assert!(pci_is_multifunction(0x81));
        assert!(!pci_is_multifunction(0x00));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::process::ProcessId;
    use crate::kernel::thread::{CpuContext, PrivilegeMode, ThreadId};

    #[test]
    fn thread_slice_handoff_carries_scheduler_selected_context() {
        let mut context = CpuContext::new(0x400000, 0x7fff_ffff_fff0, PrivilegeMode::User);
        let context_ptr = core::ptr::addr_of_mut!(context) as usize;
        let mut handoff = ThreadSliceRunContext {
            core_index: 1,
            thread: ThreadId::new(42),
            process: ProcessId::new(7),
            address_space_root: 0x1234_5000,
            kernel_stack_top: 0xffff_8000_0000_8000,
            context: &mut context,
        };

        enter_thread_slice(&mut handoff);

        assert_eq!(handoff.core_index, 1);
        assert_eq!(handoff.thread, ThreadId::new(42));
        assert_eq!(handoff.process, ProcessId::new(7));
        assert_eq!(handoff.address_space_root, 0x1234_5000);
        assert_eq!(handoff.kernel_stack_top, 0xffff_8000_0000_8000);
        assert_eq!(
            core::ptr::addr_of_mut!(*handoff.context) as usize,
            context_ptr
        );
        assert_eq!(__mirage_current_core.load(Ordering::SeqCst), usize::MAX);
        assert_eq!(__mirage_current_thread.load(Ordering::SeqCst), 0);
        assert_eq!(CURRENT_CONTEXT.load(Ordering::SeqCst), 0);
    }
}
