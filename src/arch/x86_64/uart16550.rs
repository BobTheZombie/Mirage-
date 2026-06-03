//! UART 16550 serial console driver for the legacy COM1 port.

use crate::arch::x86_64::io::{inb, outb};
use crate::kernel::device::{DeviceDriver, DeviceError, DeviceKind};
use crate::kernel::sync::SpinLock;
use crate::subkernel::{DeviceSecurity, SecurityClass};

const COM1: u16 = 0x3f8;
const DATA: u16 = 0;
const INTERRUPT_ENABLE: u16 = 1;
const FIFO_CONTROL: u16 = 2;
const LINE_CONTROL: u16 = 3;
const MODEM_CONTROL: u16 = 4;
const LINE_STATUS: u16 = 5;
const DLAB: u8 = 0x80;
const LCR_8N1: u8 = 0x03;
const LSR_DATA_READY: u8 = 0x01;
const LSR_TRANSMIT_EMPTY: u8 = 0x20;

pub struct Uart16550Driver {
    state: SpinLock<UartState>,
}

#[derive(Clone, Copy)]
struct UartState {
    initialised: bool,
}

impl Uart16550Driver {
    pub const fn new() -> Self {
        Self {
            state: SpinLock::new(UartState { initialised: false }),
        }
    }

    fn ensure_initialised(&self) {
        let mut state = self.state.lock();
        if state.initialised {
            return;
        }
        unsafe {
            outb(COM1 + INTERRUPT_ENABLE, 0x00);
            outb(COM1 + LINE_CONTROL, DLAB);
            outb(COM1 + DATA, 0x03); // 38400 baud divisor low byte.
            outb(COM1 + INTERRUPT_ENABLE, 0x00);
            outb(COM1 + LINE_CONTROL, LCR_8N1);
            outb(COM1 + FIFO_CONTROL, 0xc7); // Enable FIFO, clear RX/TX, 14-byte threshold.
            outb(COM1 + MODEM_CONTROL, 0x0b); // IRQs enabled, RTS/DSR set.
        }
        state.initialised = true;
    }

    fn line_status(&self) -> u8 {
        unsafe { inb(COM1 + LINE_STATUS) }
    }
}

impl DeviceDriver for Uart16550Driver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::SerialConsole
    }

    fn name(&self) -> &'static str {
        "uart16550-com1"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Internal, false)
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        self.ensure_initialised();
        let mut count = 0usize;
        while count < buffer.len() && self.line_status() & LSR_DATA_READY != 0 {
            buffer[count] = unsafe { inb(COM1 + DATA) };
            count += 1;
        }
        Ok(count)
    }

    fn write(&self, data: &[u8]) -> Result<usize, DeviceError> {
        self.ensure_initialised();
        for &byte in data {
            let mut spins = 0usize;
            while self.line_status() & LSR_TRANSMIT_EMPTY == 0 && spins < 100_000 {
                core::hint::spin_loop();
                spins += 1;
            }
            unsafe { outb(COM1 + DATA, byte) };
        }
        Ok(data.len())
    }
}

pub static UART16550_COM1_DRIVER: Uart16550Driver = Uart16550Driver::new();
