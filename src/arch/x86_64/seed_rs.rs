//! x86_64/QEMU seed-rs handoff layer.
//!
//! Limine still loads Mirage and enters `_start`; seed-rs is Mirage's first owned
//! handoff boundary after the minimal assembly stub.  This module deliberately
//! uses only raw COM1 port I/O and stack locals so it can report progress before
//! the normal console, allocator, framebuffer, supervisor, or MTSS paths exist.
//!
//! TODO: Gate the temporary `[seed-rs NN]` serial markers behind
//! CONFIG_MIRAGE_VERBOSE_BOOT or a `verbose-boot` feature once mirageconfig is
//! complete enough to drive that boot-time policy.

use core::arch::asm;

use crate::arch::x86_64::boot::{self, BootInfo, KernelSections};
use crate::boot as limine;
use crate::kernel::boot_phase::{
    boot_phase_ok, boot_phase_start, boot_register_compiled_subsystems, BootPhase,
};

const COM1: u16 = 0x3f8;
const COM1_INTERRUPT_ENABLE: u16 = COM1 + 1;
const COM1_FIFO_CONTROL: u16 = COM1 + 2;
const COM1_LINE_CONTROL: u16 = COM1 + 3;
const COM1_MODEM_CONTROL: u16 = COM1 + 4;
const COM1_LINE_STATUS: u16 = COM1 + 5;
const COM1_TRANSMITTER_EMPTY: u8 = 0x20;

extern "Rust" {
    fn kernel_main(boot_info: BootInfo) -> !;
}

/// Mirage-owned x86_64 handoff after Limine enters the kernel ELF.
pub unsafe fn x86_64_handoff() -> ! {
    seed_com1_init();

    boot::clear_bss();
    boot_register_compiled_subsystems();
    boot_phase_start(BootPhase::SeedRs);
    seed_com1_write_str("[seed-rs 01] entered seed entry\r\n");
    seed_com1_write_str("[seed-rs 02] bss cleared\r\n");

    let sections = KernelSections::from_linker();
    seed_com1_write_str("[seed-rs 03] linker sections captured\r\n");

    let raw_boot = limine::snapshot();
    seed_com1_write_str("[seed-rs 04] limine snapshot captured\r\n");

    boot_phase_start(BootPhase::BootInfo);
    let boot_info = BootInfo::from_limine(raw_boot, sections);
    boot_phase_ok(BootPhase::BootInfo);
    seed_com1_write_str("[seed-rs 05] bootinfo constructed\r\n");

    boot_phase_ok(BootPhase::SeedRs);
    seed_com1_write_str("[seed-rs 06] calling kernel_main\r\n");
    kernel_main(boot_info)
}

/// Initialize COM1 defensively for seed-rs raw serial diagnostics.
pub unsafe fn seed_com1_init() {
    // Keep the seed path independent of interrupt setup.  COM1 interrupts are
    // disabled at the UART and CPU interrupt delivery is disabled until the
    // normal architecture path chooses to enable it.
    asm!("cli", options(nomem, nostack, preserves_flags));
    seed_enable_sse();

    outb(COM1_INTERRUPT_ENABLE, 0x00);
    outb(COM1_LINE_CONTROL, 0x80);
    // Divisor 3 with the standard 1.8432 MHz UART clock gives 38400 baud.
    outb(COM1, 0x03);
    outb(COM1_INTERRUPT_ENABLE, 0x00);
    // 8 data bits, no parity, one stop bit.
    outb(COM1_LINE_CONTROL, 0x03);
    // Enable FIFO, clear RX/TX queues, 14-byte threshold.
    outb(COM1_FIFO_CONTROL, 0xc7);
    // Data terminal ready, request to send, and OUT2 for PC-compatible UARTs.
    outb(COM1_MODEM_CONTROL, 0x0b);
}

/// Enable SSE/SSE2 before optimized Rust code can emit XMM moves in the seed path.
unsafe fn seed_enable_sse() {
    let mut cr0: u64;
    asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
    cr0 &= !(1 << 2);
    cr0 |= 1 << 1;
    asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack, preserves_flags));

    let mut cr4: u64;
    asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
    cr4 |= (1 << 9) | (1 << 10);
    asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack, preserves_flags));
}

/// Write one byte to COM1 using only raw x86_64 port I/O.
pub unsafe fn seed_com1_write_byte(byte: u8) {
    seed_com1_init();
    seed_com1_write_byte_raw(byte);
}

/// Write a string to COM1 without formatting or heap use.
pub unsafe fn seed_com1_write_str(s: &str) {
    seed_com1_init();
    for byte in s.bytes() {
        seed_com1_write_byte_raw(byte);
    }
}

/// Emit a compact numeric seed marker on COM1.
pub unsafe fn seed_marker(id: u8) {
    seed_com1_init();
    seed_com1_write_str_raw("[seed-rs ");
    seed_com1_write_byte_raw(hex_digit(id >> 4));
    seed_com1_write_byte_raw(hex_digit(id & 0x0f));
    seed_com1_write_str_raw("]\r\n");
}

unsafe fn seed_com1_write_str_raw(s: &str) {
    for byte in s.bytes() {
        seed_com1_write_byte_raw(byte);
    }
}

unsafe fn seed_com1_write_byte_raw(byte: u8) {
    wait_for_transmitter_empty();
    outb(COM1, byte);
}

unsafe fn wait_for_transmitter_empty() {
    let mut spins = 0usize;
    while inb(COM1_LINE_STATUS) & COM1_TRANSMITTER_EMPTY == 0 && spins < 100_000 {
        core::hint::spin_loop();
        spins += 1;
    }
}

const fn hex_digit(nibble: u8) -> u8 {
    match nibble & 0x0f {
        0..=9 => b'0' + (nibble & 0x0f),
        value => b'a' + (value - 10),
    }
}

#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

#[inline(always)]
unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}
