//! Interrupt Descriptor Table and early exception/IRQ handlers.

use core::sync::atomic::{AtomicU64, Ordering};

use super::{gdt, msr, pic};

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

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry::missing(); IDT_ENTRIES];

#[cfg(not(test))]
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

#[cfg(not(test))]
core::arch::global_asm!(
    r#"
    .macro PUSH_SCRATCH
        push rax
        push rcx
        push rdx
        push rsi
        push rdi
        push r8
        push r9
        push r10
        push r11
    .endm

    .macro POP_SCRATCH
        pop r11
        pop r10
        pop r9
        pop r8
        pop rdi
        pop rsi
        pop rdx
        pop rcx
        pop rax
    .endm

    .macro ISR_NOERR name, vector
    .global \name
    .type \name,@function
\name:
        push 0
        push \vector
        jmp __mirage_isr_common
    .endm

    .macro ISR_ERR name, vector
    .global \name
    .type \name,@function
\name:
        push \vector
        jmp __mirage_isr_common
    .endm

    ISR_NOERR __mirage_isr_divide_error, 0
    ISR_NOERR __mirage_isr_debug, 1
    ISR_NOERR __mirage_isr_non_maskable, 2
    ISR_NOERR __mirage_isr_breakpoint, 3
    ISR_NOERR __mirage_isr_overflow, 4
    ISR_NOERR __mirage_isr_bound_range, 5
    ISR_NOERR __mirage_isr_invalid_opcode, 6
    ISR_NOERR __mirage_isr_device_not_available, 7
    ISR_ERR   __mirage_isr_double_fault, 8
    ISR_ERR   __mirage_isr_invalid_tss, 10
    ISR_ERR   __mirage_isr_segment_not_present, 11
    ISR_ERR   __mirage_isr_stack_segment_fault, 12
    ISR_ERR   __mirage_isr_general_protection, 13
    ISR_ERR   __mirage_isr_page_fault, 14
    ISR_NOERR __mirage_isr_x87, 16
    ISR_ERR   __mirage_isr_alignment_check, 17
    ISR_NOERR __mirage_isr_machine_check, 18
    ISR_NOERR __mirage_isr_simd, 19
    ISR_NOERR __mirage_isr_virtualization, 20
    ISR_NOERR __mirage_isr_timer, 32
    ISR_NOERR __mirage_isr_syscall_trap, 128

    .global __mirage_isr_common
    .type __mirage_isr_common,@function
__mirage_isr_common:
        PUSH_SCRATCH
        mov rdi, [rsp + 72]
        mov rsi, [rsp + 80]
        call __mirage_rust_interrupt_dispatch
        POP_SCRATCH
        add rsp, 16
        iretq

    .global __mirage_syscall_entry
    .type __mirage_syscall_entry,@function
__mirage_syscall_entry:
        push 0
        push 128
        jmp __mirage_isr_common
"#
);

#[cfg(test)]
extern "C" fn __mirage_isr_divide_error() {}
#[cfg(test)]
extern "C" fn __mirage_isr_debug() {}
#[cfg(test)]
extern "C" fn __mirage_isr_non_maskable() {}
#[cfg(test)]
extern "C" fn __mirage_isr_breakpoint() {}
#[cfg(test)]
extern "C" fn __mirage_isr_overflow() {}
#[cfg(test)]
extern "C" fn __mirage_isr_bound_range() {}
#[cfg(test)]
extern "C" fn __mirage_isr_invalid_opcode() {}
#[cfg(test)]
extern "C" fn __mirage_isr_device_not_available() {}
#[cfg(test)]
extern "C" fn __mirage_isr_double_fault() {}
#[cfg(test)]
extern "C" fn __mirage_isr_invalid_tss() {}
#[cfg(test)]
extern "C" fn __mirage_isr_segment_not_present() {}
#[cfg(test)]
extern "C" fn __mirage_isr_stack_segment_fault() {}
#[cfg(test)]
extern "C" fn __mirage_isr_general_protection() {}
#[cfg(test)]
extern "C" fn __mirage_isr_page_fault() {}
#[cfg(test)]
extern "C" fn __mirage_isr_x87() {}
#[cfg(test)]
extern "C" fn __mirage_isr_alignment_check() {}
#[cfg(test)]
extern "C" fn __mirage_isr_machine_check() {}
#[cfg(test)]
extern "C" fn __mirage_isr_simd() {}
#[cfg(test)]
extern "C" fn __mirage_isr_virtualization() {}
#[cfg(test)]
extern "C" fn __mirage_isr_timer() {}
#[cfg(test)]
extern "C" fn __mirage_isr_syscall_trap() {}
#[cfg(test)]
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
    #[cfg(not(test))]
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
    LAST_EXCEPTION_VECTOR.store(
        (vector << 32) | (error_code & 0xffff_ffff),
        Ordering::SeqCst,
    );

    match vector as u8 {
        PAGE_FAULT_VECTOR => LAST_PAGE_FAULT_ADDRESS.store(read_cr2(), Ordering::SeqCst),
        vector if vector == pic::TIMER_VECTOR => {
            TIMER_TICKS.fetch_add(1, Ordering::SeqCst);
            pic::end_of_interrupt(vector);
        }
        SYSCALL_TRAP_VECTOR => {
            SYSCALL_TRAPS.fetch_add(1, Ordering::SeqCst);
        }
        _ => {}
    }
}

fn read_cr2() -> u64 {
    #[cfg(not(test))]
    unsafe {
        let value: u64;
        core::arch::asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags));
        value
    }

    #[cfg(test)]
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
