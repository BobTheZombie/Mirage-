//! Global Descriptor Table and Task State Segment setup.

use core::mem::size_of;

use crate::kernel::cpu::MAX_CORES;

pub const KERNEL_CODE_SELECTOR: u16 = 0x08;
pub const KERNEL_DATA_SELECTOR: u16 = 0x10;
pub const USER_CODE_SELECTOR: u16 = 0x1b;
pub const USER_DATA_SELECTOR: u16 = 0x23;
pub const TSS_SELECTOR: u16 = 0x28;

pub const DOUBLE_FAULT_IST_INDEX: u8 = 1;
pub const PAGE_FAULT_IST_INDEX: u8 = 2;
pub const INTERRUPT_IST_INDEX: u8 = 3;

const STACK_SIZE: usize = 16 * 1024;
const GDT_ENTRIES: usize = 7;

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct TaskStateSegment {
    _reserved0: u32,
    privilege_stack_table: [u64; 3],
    _reserved1: u64,
    interrupt_stack_table: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    io_map_base: u16,
}

impl TaskStateSegment {
    const fn new() -> Self {
        Self {
            _reserved0: 0,
            privilege_stack_table: [0; 3],
            _reserved1: 0,
            interrupt_stack_table: [0; 7],
            _reserved2: 0,
            _reserved3: 0,
            io_map_base: size_of::<TaskStateSegment>() as u16,
        }
    }
}

#[repr(align(16))]
#[derive(Clone, Copy)]
struct Stack([u8; STACK_SIZE]);

static mut DOUBLE_FAULT_STACK: Stack = Stack([0; STACK_SIZE]);
static mut PAGE_FAULT_STACK: Stack = Stack([0; STACK_SIZE]);
static mut INTERRUPT_STACK: Stack = Stack([0; STACK_SIZE]);
static mut KERNEL_STACKS: [Stack; MAX_CORES] = [Stack([0; STACK_SIZE]); MAX_CORES];
static mut TSS: TaskStateSegment = TaskStateSegment::new();
static mut GDT: [u64; GDT_ENTRIES] = [0; GDT_ENTRIES];

/// Build and load a GDT containing kernel/user segments and a TSS with dedicated IST stacks.
pub fn initialize() {
    unsafe {
        TSS.interrupt_stack_table[(DOUBLE_FAULT_IST_INDEX - 1) as usize] =
            stack_top(core::ptr::addr_of!(DOUBLE_FAULT_STACK));
        TSS.interrupt_stack_table[(PAGE_FAULT_IST_INDEX - 1) as usize] =
            stack_top(core::ptr::addr_of!(PAGE_FAULT_STACK));
        TSS.interrupt_stack_table[(INTERRUPT_IST_INDEX - 1) as usize] =
            stack_top(core::ptr::addr_of!(INTERRUPT_STACK));
        TSS.privilege_stack_table[0] = kernel_stack_top(0);

        GDT[0] = 0;
        GDT[1] = code_descriptor(0, true);
        GDT[2] = data_descriptor(0);
        GDT[3] = code_descriptor(3, true);
        GDT[4] = data_descriptor(3);
        let (tss_low, tss_high) = tss_descriptor(core::ptr::addr_of!(TSS));
        GDT[5] = tss_low;
        GDT[6] = tss_high;

        load();
    }
}

pub fn set_current_kernel_stack(core_index: usize) {
    #[cfg(not(test))]
    unsafe {
        TSS.privilege_stack_table[0] = kernel_stack_top(core_index);
    }

    #[cfg(test)]
    let _ = core_index;
}

pub fn kernel_stack_top(core_index: usize) -> u64 {
    let index = if core_index < MAX_CORES {
        core_index
    } else {
        0
    };
    unsafe { stack_top(core::ptr::addr_of!(KERNEL_STACKS[index])) }
}

unsafe fn stack_top(stack: *const Stack) -> u64 {
    core::ptr::addr_of!((*stack).0).cast::<u8>().add(STACK_SIZE) as u64
}

const fn code_descriptor(dpl: u64, long_mode: bool) -> u64 {
    let access = 0x9a | (dpl << 5);
    let flags = if long_mode { 0x20 } else { 0x20 };
    (access << 40) | (flags << 48)
}

const fn data_descriptor(dpl: u64) -> u64 {
    let access = 0x92 | (dpl << 5);
    (access << 40) | (0x40 << 48)
}

unsafe fn tss_descriptor(tss: *const TaskStateSegment) -> (u64, u64) {
    let base = tss as u64;
    let limit = (size_of::<TaskStateSegment>() - 1) as u64;
    let low = (limit & 0xffff)
        | ((base & 0xffff) << 16)
        | (((base >> 16) & 0xff) << 32)
        | (0x89 << 40)
        | (((limit >> 16) & 0x0f) << 48)
        | (((base >> 24) & 0xff) << 56);
    let high = base >> 32;
    (low, high)
}

unsafe fn load() {
    #[cfg(not(test))]
    {
        let pointer = DescriptorTablePointer {
            limit: (size_of::<[u64; GDT_ENTRIES]>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u64,
        };

        core::arch::asm!("lgdt [{0}]", in(reg) &pointer, options(readonly, nostack, preserves_flags));
        core::arch::asm!(
            "push {code}",
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            code = const KERNEL_CODE_SELECTOR as u64,
            out("rax") _,
        );
        core::arch::asm!(
            "mov ds, ax",
            "mov es, ax",
            "mov ss, ax",
            "mov fs, ax",
            "mov gs, ax",
            in("ax") KERNEL_DATA_SELECTOR,
            options(nostack, preserves_flags),
        );
        core::arch::asm!("ltr ax", in("ax") TSS_SELECTOR, options(nostack, preserves_flags));
    }
}
