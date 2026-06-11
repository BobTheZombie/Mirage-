//! Minimal COM1 debug output for the earliest x86_64 boot path.
//!
//! This module deliberately avoids the normal console stack. It performs direct
//! programmed I/O against the legacy COM1 UART and does not use heap allocation,
//! Rust formatting, locks, interrupts, framebuffer output, paging state, or any
//! global console registration.

use core::arch::asm;

const COM1_DATA_PORT: u16 = 0x3f8;
const COM1_INTERRUPT_ENABLE_PORT: u16 = 0x3f9;
const COM1_FIFO_CONTROL_PORT: u16 = 0x3fa;
const COM1_LINE_CONTROL_PORT: u16 = 0x3fb;
const COM1_MODEM_CONTROL_PORT: u16 = 0x3fc;
const COM1_LINE_STATUS_PORT: u16 = 0x3fd;

const LINE_CONTROL_DLAB: u8 = 0x80;
const LINE_CONTROL_8N1: u8 = 0x03;
const LINE_STATUS_TRANSMIT_EMPTY: u8 = 0x20;

/// Write one byte to the legacy COM1 serial port.
///
/// # Safety
///
/// This performs privileged x86 port I/O and is only valid while running at a
/// privilege level where direct access to the legacy COM1 UART ports is allowed.
pub unsafe fn com1_write_byte(byte: u8) {
    com1_init();
    com1_write_byte_initialised(byte);
}

/// Write a UTF-8 string to the legacy COM1 serial port byte-by-byte.
///
/// Newline bytes are expanded to CRLF so simple boot traces are readable on
/// common serial consoles. No formatting or allocation is performed.
///
/// # Safety
///
/// This performs privileged x86 port I/O and is only valid while running at a
/// privilege level where direct access to the legacy COM1 UART ports is allowed.
pub unsafe fn com1_write_str(s: &str) {
    com1_init();

    for byte in s.bytes() {
        if byte == b'\n' {
            com1_write_byte_initialised(b'\r');
        }
        com1_write_byte_initialised(byte);
    }
}

/// Emit a compact boot progress marker to COM1.
///
/// The marker format is intentionally fixed and allocation-free:
/// `[MIRAGE BOOT NN]\r\n`.
///
/// # Safety
///
/// This performs privileged x86 port I/O and is only valid while running at a
/// privilege level where direct access to the legacy COM1 UART ports is allowed.
pub unsafe fn boot_marker(id: u8) {
    com1_init();

    for byte in b"[MIRAGE BOOT " {
        com1_write_byte_initialised(*byte);
    }
    com1_write_byte_initialised(b'0' + ((id / 10) % 10));
    com1_write_byte_initialised(b'0' + (id % 10));
    com1_write_byte_initialised(b']');
    com1_write_byte_initialised(b'\r');
    com1_write_byte_initialised(b'\n');
}

unsafe fn com1_init() {
    outb(COM1_INTERRUPT_ENABLE_PORT, 0x00);
    outb(COM1_LINE_CONTROL_PORT, LINE_CONTROL_DLAB);
    outb(COM1_DATA_PORT, 0x03);
    outb(COM1_INTERRUPT_ENABLE_PORT, 0x00);
    outb(COM1_LINE_CONTROL_PORT, LINE_CONTROL_8N1);
    outb(COM1_FIFO_CONTROL_PORT, 0xc7);
    outb(COM1_MODEM_CONTROL_PORT, 0x03);
}

unsafe fn com1_write_byte_initialised(byte: u8) {
    wait_for_transmit_empty();
    outb(COM1_DATA_PORT, byte);
}

unsafe fn wait_for_transmit_empty() {
    while inb(COM1_LINE_STATUS_PORT) & LINE_STATUS_TRANSMIT_EMPTY == 0 {
        core::hint::spin_loop();
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
