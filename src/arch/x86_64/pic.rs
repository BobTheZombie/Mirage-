//! Legacy 8259 PIC setup for early timer interrupts.

const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xa0;
const PIC2_DATA: u16 = 0xa1;

const ICW1_INIT: u8 = 0x10;
const ICW1_ICW4: u8 = 0x01;
const ICW4_8086: u8 = 0x01;
const PIC_EOI: u8 = 0x20;

pub const MASTER_OFFSET: u8 = 32;
pub const SLAVE_OFFSET: u8 = 40;
pub const TIMER_VECTOR: u8 = MASTER_OFFSET;

#[inline(always)]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}

#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags));
    value
}

#[inline(always)]
unsafe fn wait() {
    outb(0x80, 0);
}

/// Remap the PICs away from CPU exception vectors and unmask the PIT timer IRQ.
pub fn initialize() {
    #[cfg(not(test))]
    unsafe {
        let master_mask = inb(PIC1_DATA);
        let slave_mask = inb(PIC2_DATA);

        outb(PIC1_COMMAND, ICW1_INIT | ICW1_ICW4);
        wait();
        outb(PIC2_COMMAND, ICW1_INIT | ICW1_ICW4);
        wait();

        outb(PIC1_DATA, MASTER_OFFSET);
        wait();
        outb(PIC2_DATA, SLAVE_OFFSET);
        wait();

        outb(PIC1_DATA, 4);
        wait();
        outb(PIC2_DATA, 2);
        wait();

        outb(PIC1_DATA, ICW4_8086);
        wait();
        outb(PIC2_DATA, ICW4_8086);
        wait();

        outb(PIC1_DATA, master_mask & !1);
        outb(PIC2_DATA, slave_mask);
    }
}

/// Notify the PIC that an interrupt vector has been handled.
pub fn end_of_interrupt(vector: u8) {
    #[cfg(not(test))]
    unsafe {
        if vector >= SLAVE_OFFSET {
            outb(PIC2_COMMAND, PIC_EOI);
        }
        outb(PIC1_COMMAND, PIC_EOI);
    }

    #[cfg(test)]
    let _ = vector;
}
