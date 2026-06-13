//! Interrupt Descriptor Table and early exception/IRQ handlers.

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(not(any(test, feature = "qfs-std")))]
use super::interrupts;
use super::{gdt, msr, pic};
use crate::kernel::thread::{CpuContext, PrivilegeMode};

pub const DOUBLE_FAULT_VECTOR: u8 = 8;
pub const PAGE_FAULT_VECTOR: u8 = 14;
pub const SYSCALL_TRAP_VECTOR: u8 = 0x80;

const IDT_ENTRIES: usize = 256;
const PRESENT: u8 = 1 << 7;
const INTERRUPT_GATE: u8 = 0x0e;
const TRAP_GATE: u8 = 0x0f;
const DPL3: u8 = 3 << 5;

static LAST_EXCEPTION_VECTOR: AtomicU64 = AtomicU64::new(0);
static LAST_PAGE_FAULT_ADDRESS: AtomicU64 = AtomicU64::new(0);
static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);
static SYSCALL_TRAPS: AtomicU64 = AtomicU64::new(0);

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    options: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            options: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }

    fn set(&mut self, handler: usize, selector: u16, ist: u8, options: u8) {
        self.offset_low = handler as u16;
        self.selector = selector;
        self.ist = ist & 0x07;
        self.options = options;
        self.offset_mid = (handler >> 16) as u16;
        self.offset_high = (handler >> 32) as u32;
        self.reserved = 0;
    }
}

#[cfg(not(any(test, feature = "qfs-std")))]
#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry::missing(); IDT_ENTRIES];

#[cfg(not(any(test, feature = "qfs-std")))]
extern "C" {
    fn __mirage_isr_divide_error();
    fn __mirage_isr_debug();
    fn __mirage_isr_non_maskable();
    fn __mirage_isr_breakpoint();
    fn __mirage_isr_overflow();
    fn __mirage_isr_bound_range();
    fn __mirage_isr_invalid_opcode();
    fn __mirage_isr_device_not_available();
    fn __mirage_isr_double_fault();
    fn __mirage_isr_invalid_tss();
    fn __mirage_isr_segment_not_present();
    fn __mirage_isr_stack_segment_fault();
    fn __mirage_isr_general_protection();
    fn __mirage_isr_page_fault();
    fn __mirage_isr_x87();
    fn __mirage_isr_alignment_check();
    fn __mirage_isr_machine_check();
    fn __mirage_isr_simd();
    fn __mirage_isr_virtualization();
    fn __mirage_isr_timer();
    fn __mirage_isr_syscall_trap();
    fn __mirage_syscall_entry();
}

#[cfg(not(any(test, feature = "qfs-std")))]
core::arch::global_asm!(include_str!("entry.S"));

#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_divide_error() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_debug() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_non_maskable() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_breakpoint() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_overflow() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_bound_range() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_invalid_opcode() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_device_not_available() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_double_fault() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_invalid_tss() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_segment_not_present() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_stack_segment_fault() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_general_protection() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_page_fault() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_x87() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_alignment_check() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_machine_check() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_simd() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_virtualization() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_timer() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_isr_syscall_trap() {}
#[cfg(any(test, feature = "qfs-std"))]
extern "C" fn __mirage_syscall_entry() {}

/// Build and load the IDT, then enable the CPU syscall entry point.
pub fn initialize() {
    unsafe {
        let kernel_gate = PRESENT | INTERRUPT_GATE;
        let user_trap_gate = PRESENT | TRAP_GATE | DPL3;

        set_gate(0, __mirage_isr_divide_error as usize, 0, kernel_gate);
        set_gate(1, __mirage_isr_debug as usize, 0, kernel_gate);
        set_gate(
            2,
            __mirage_isr_non_maskable as usize,
            gdt::INTERRUPT_IST_INDEX,
            kernel_gate,
        );
        set_gate(3, __mirage_isr_breakpoint as usize, 0, user_trap_gate);
        set_gate(4, __mirage_isr_overflow as usize, 0, user_trap_gate);
        set_gate(5, __mirage_isr_bound_range as usize, 0, kernel_gate);
        set_gate(6, __mirage_isr_invalid_opcode as usize, 0, kernel_gate);
        set_gate(
            7,
            __mirage_isr_device_not_available as usize,
            0,
            kernel_gate,
        );
        set_gate(
            DOUBLE_FAULT_VECTOR,
            __mirage_isr_double_fault as usize,
            gdt::DOUBLE_FAULT_IST_INDEX,
            kernel_gate,
        );
        set_gate(10, __mirage_isr_invalid_tss as usize, 0, kernel_gate);
        set_gate(
            11,
            __mirage_isr_segment_not_present as usize,
            0,
            kernel_gate,
        );
        set_gate(
            12,
            __mirage_isr_stack_segment_fault as usize,
            0,
            kernel_gate,
        );
        set_gate(13, __mirage_isr_general_protection as usize, 0, kernel_gate);
        set_gate(
            PAGE_FAULT_VECTOR,
            __mirage_isr_page_fault as usize,
            gdt::PAGE_FAULT_IST_INDEX,
            kernel_gate,
        );
        set_gate(16, __mirage_isr_x87 as usize, 0, kernel_gate);
        set_gate(17, __mirage_isr_alignment_check as usize, 0, kernel_gate);
        set_gate(
            18,
            __mirage_isr_machine_check as usize,
            gdt::INTERRUPT_IST_INDEX,
            kernel_gate,
        );
        set_gate(19, __mirage_isr_simd as usize, 0, kernel_gate);
        set_gate(20, __mirage_isr_virtualization as usize, 0, kernel_gate);
        set_gate(
            pic::TIMER_VECTOR,
            __mirage_isr_timer as usize,
            gdt::INTERRUPT_IST_INDEX,
            kernel_gate,
        );
        set_gate(
            SYSCALL_TRAP_VECTOR,
            __mirage_isr_syscall_trap as usize,
            0,
            user_trap_gate,
        );

        load();
        msr::enable_syscall_entry(
            __mirage_syscall_entry as usize,
            gdt::KERNEL_CODE_SELECTOR,
            gdt::USER_CODE_SELECTOR,
        );
    }
}

unsafe fn set_gate(vector: u8, handler: usize, ist: u8, options: u8) {
    IDT[vector as usize].set(handler, gdt::KERNEL_CODE_SELECTOR, ist, options);
}

unsafe fn load() {
    #[cfg(not(any(test, feature = "qfs-std")))]
    {
        let pointer = DescriptorTablePointer {
            limit: (core::mem::size_of::<[IdtEntry; IDT_ENTRIES]>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u64,
        };
        core::arch::asm!("lidt [{0}]", in(reg) &pointer, options(readonly, nostack, preserves_flags));
    }
}

#[no_mangle]
extern "C" fn __mirage_rust_interrupt_dispatch(vector: u64, error_code: u64) {
    dispatch_interrupt(vector, error_code);
}

pub(crate) fn dispatch_interrupt(vector: u64, error_code: u64) {
    dispatch_interrupt_with_context(vector, error_code, None);
}

pub(crate) fn dispatch_interrupt_frame(context: &CpuContext) {
    dispatch_interrupt_with_context(context.trap_vector, context.error_code, Some(context));
}

fn dispatch_interrupt_with_context(vector: u64, error_code: u64, context: Option<&CpuContext>) {
    LAST_EXCEPTION_VECTOR.store(
        (vector << 32) | (error_code & 0xffff_ffff),
        Ordering::SeqCst,
    );

    match vector as u8 {
        vector if vector == pic::TIMER_VECTOR => {
            TIMER_TICKS.fetch_add(1, Ordering::SeqCst);
            pic::end_of_interrupt(vector);
        }
        SYSCALL_TRAP_VECTOR => {
            SYSCALL_TRAPS.fetch_add(1, Ordering::SeqCst);
        }
        3 => print_exception_diagnostics("breakpoint", vector, error_code, context, None),
        DOUBLE_FAULT_VECTOR => {
            print_exception_diagnostics("double fault", vector, error_code, context, None);
            crate::kprintln!("x86_64: double fault is fatal; halting safely");
            halt_safely();
        }
        13 => {
            print_exception_diagnostics(
                "general protection fault",
                vector,
                error_code,
                context,
                None,
            );
            crate::kprintln!("x86_64: general protection fault is fatal; halting safely");
            halt_safely();
        }
        PAGE_FAULT_VECTOR => {
            let cr2 = read_cr2();
            LAST_PAGE_FAULT_ADDRESS.store(cr2, Ordering::SeqCst);
            print_exception_diagnostics("page fault", vector, error_code, context, Some(cr2));
            print_page_fault_decode(error_code);
            print_page_walk(cr2);
            if context.is_none_or(|frame| frame.privilege_mode == PrivilegeMode::Kernel) {
                crate::kprintln!("x86_64: kernel page fault is fatal; halting safely");
                halt_safely();
            }
        }
        _ => {}
    }
}

fn print_exception_diagnostics(
    name: &str,
    vector: u64,
    error_code: u64,
    context: Option<&CpuContext>,
    cr2: Option<u64>,
) {
    crate::kprintln!(
        "x86_64 exception: {} vector={} error_code={:#x}",
        name,
        vector,
        error_code
    );

    if let Some(cr2) = cr2 {
        crate::kprintln!("  cr2={:#018x}", cr2);
    }

    if let Some(context) = context {
        crate::kprintln!(
            "  rip={:#018x} rsp={:#018x} rflags={:#018x} cs={:#x} ss={:#x} mode={:?}",
            context.rip,
            context.rsp,
            context.rflags,
            context.cs,
            context.ss,
            context.privilege_mode
        );
        crate::kprintln!(
            "  rax={:#018x} rbx={:#018x} rcx={:#018x} rdx={:#018x}",
            context.rax,
            context.rbx,
            context.rcx,
            context.rdx
        );
        crate::kprintln!(
            "  rsi={:#018x} rdi={:#018x} rbp={:#018x}",
            context.rsi,
            context.rdi,
            context.rbp
        );
        crate::kprintln!(
            "  r8={:#018x} r9={:#018x} r10={:#018x} r11={:#018x}",
            context.r8,
            context.r9,
            context.r10,
            context.r11
        );
        crate::kprintln!(
            "  r12={:#018x} r13={:#018x} r14={:#018x} r15={:#018x}",
            context.r12,
            context.r13,
            context.r14,
            context.r15
        );
        crate::kprintln!(
            "  fs={:#x} gs={:#x} fs_base={:#018x} gs_base={:#018x}",
            context.fs,
            context.gs,
            context.fs_base,
            context.gs_base
        );
    } else {
        crate::kprintln!("  register frame unavailable");
    }
}

fn print_page_fault_decode(error_code: u64) {
    crate::kprintln!(
        "  page-fault bits: present={} write={} user={} reserved={} instruction_fetch={} protection_key={} shadow_stack={} sgx={}",
        (error_code & (1 << 0)) != 0,
        (error_code & (1 << 1)) != 0,
        (error_code & (1 << 2)) != 0,
        (error_code & (1 << 3)) != 0,
        (error_code & (1 << 4)) != 0,
        (error_code & (1 << 5)) != 0,
        (error_code & (1 << 6)) != 0,
        (error_code & (1 << 15)) != 0
    );
}

fn print_page_walk(virtual_address: u64) {
    match crate::arch::x86_64::paging::walk_kernel_page_tables(virtual_address) {
        Some(walk) => {
            crate::kprintln!("  cr3={:#018x}", walk.cr3);
            crate::kprintln!(
                "  pml4e={:#018x} pdpte={:#018x} pde={:#018x} pte={:#018x}",
                walk.pml4e,
                walk.pdpte,
                walk.pde,
                walk.pte
            );
            match walk.physical {
                Some(physical) => {
                    crate::kprintln!("  translated={:#018x}", physical);
                }
                None => {
                    crate::kprintln!("  translated=<not present>");
                }
            }
        }
        None => {
            crate::kprintln!("  page table walk unavailable");
        }
    }
}

#[cfg(not(any(test, feature = "qfs-std")))]
fn halt_safely() -> ! {
    interrupts::halt_forever()
}

#[cfg(any(test, feature = "qfs-std"))]
fn halt_safely() -> ! {
    panic!("fatal x86_64 trap halted safely")
}

fn read_cr2() -> u64 {
    #[cfg(not(any(test, feature = "qfs-std")))]
    unsafe {
        let value: u64;
        core::arch::asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags));
        value
    }

    #[cfg(any(test, feature = "qfs-std"))]
    {
        0
    }
}

pub fn timer_ticks() -> u64 {
    TIMER_TICKS.load(Ordering::SeqCst)
}

pub fn last_page_fault_address() -> u64 {
    LAST_PAGE_FAULT_ADDRESS.load(Ordering::SeqCst)
}
