//! Real x86_64 i8042 PS/2 controller bring-up for built-in laptop keyboards.

use crate::arch::x86_64::io::{inb, io_wait, outb};

pub const DATA_PORT: u16 = 0x60;
pub const STATUS_PORT: u16 = 0x64;
pub const COMMAND_PORT: u16 = 0x64;

const STATUS_OUTPUT_FULL: u8 = 0x01;
const STATUS_INPUT_FULL: u8 = 0x02;
const STATUS_AUX_DATA: u8 = 0x20;

const CMD_READ_CONFIG: u8 = 0x20;
const CMD_WRITE_CONFIG: u8 = 0x60;
const CMD_DISABLE_FIRST: u8 = 0xad;
const CMD_ENABLE_FIRST: u8 = 0xae;
const CMD_DISABLE_SECOND: u8 = 0xa7;
const CMD_TEST_SECOND: u8 = 0xa9;
const CMD_TEST_FIRST: u8 = 0xab;
const CMD_SELF_TEST: u8 = 0xaa;

const CONFIG_IRQ1: u8 = 1 << 0;
const CONFIG_IRQ12: u8 = 1 << 1;
const CONFIG_FIRST_CLOCK_DISABLED: u8 = 1 << 4;
const CONFIG_SECOND_CLOCK_DISABLED: u8 = 1 << 5;
const CONFIG_TRANSLATION: u8 = 1 << 6;

pub const PS2_ACK: u8 = 0xfa;
pub const PS2_RESEND: u8 = 0xfe;
pub const PS2_BAT_OK: u8 = 0xaa;

const WAIT_LIMIT: usize = 100_000;
const FLUSH_LIMIT: usize = 64;
const RETRIES: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum I8042Error {
    Timeout,
    SelfTestFailed(u8),
    PortTestFailed(u8),
    DeviceError(u8),
    ResendLimit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct I8042InitResult {
    pub config: u8,
    pub translated: bool,
    pub second_port_present: bool,
}

pub struct I8042Controller;

impl I8042Controller {
    pub const fn new() -> Self {
        Self
    }

    pub fn initialize(
        &self,
        irq_mode: bool,
        prefer_set2: bool,
    ) -> Result<I8042InitResult, I8042Error> {
        self.write_command(CMD_DISABLE_FIRST)?;
        let _ = self.write_command(CMD_DISABLE_SECOND);
        self.flush_output();

        let mut config = self.read_config()?;
        config &= !(CONFIG_IRQ1 | CONFIG_IRQ12);
        config |= CONFIG_FIRST_CLOCK_DISABLED;
        self.write_config(config)?;

        self.write_command(CMD_SELF_TEST)?;
        let self_test = self.read_data()?;
        if self_test != 0x55 {
            return Err(I8042Error::SelfTestFailed(self_test));
        }
        self.write_config(config)?;

        self.write_command(CMD_TEST_FIRST)?;
        let first_test = self.read_data()?;
        if first_test != 0x00 {
            return Err(I8042Error::PortTestFailed(first_test));
        }

        let second_port_present = match self
            .write_command(CMD_TEST_SECOND)
            .and_then(|_| self.read_data())
        {
            Ok(0x00) => true,
            _ => false,
        };

        config &= !CONFIG_FIRST_CLOCK_DISABLED;
        if prefer_set2 {
            config &= !CONFIG_TRANSLATION;
        }
        if !second_port_present {
            config |= CONFIG_SECOND_CLOCK_DISABLED;
        }
        if irq_mode {
            config |= CONFIG_IRQ1;
        }
        self.write_config(config)?;
        self.write_command(CMD_ENABLE_FIRST)?;
        self.flush_output();

        Ok(I8042InitResult {
            config,
            translated: config & CONFIG_TRANSLATION != 0,
            second_port_present,
        })
    }

    pub fn read_config(&self) -> Result<u8, I8042Error> {
        self.write_command(CMD_READ_CONFIG)?;
        self.read_data()
    }

    pub fn write_config(&self, config: u8) -> Result<(), I8042Error> {
        self.write_command(CMD_WRITE_CONFIG)?;
        self.write_data(config)
    }

    pub fn data_available(&self) -> bool {
        unsafe { inb(STATUS_PORT) & STATUS_OUTPUT_FULL != 0 }
    }

    pub fn status(&self) -> u8 {
        unsafe { inb(STATUS_PORT) }
    }

    pub fn read_data(&self) -> Result<u8, I8042Error> {
        self.wait_output_full()?;
        Ok(unsafe { inb(DATA_PORT) })
    }

    pub fn write_data(&self, value: u8) -> Result<(), I8042Error> {
        self.wait_input_empty()?;
        unsafe { outb(DATA_PORT, value) };
        io_wait();
        Ok(())
    }

    pub fn write_command(&self, command: u8) -> Result<(), I8042Error> {
        self.wait_input_empty()?;
        unsafe { outb(COMMAND_PORT, command) };
        io_wait();
        Ok(())
    }

    pub fn send_device_command(&self, command: u8) -> Result<(), I8042Error> {
        let mut attempt = 0usize;
        while attempt < RETRIES {
            self.write_data(command)?;
            match self.read_data()? {
                PS2_ACK => return Ok(()),
                PS2_RESEND => attempt += 1,
                byte => return Err(I8042Error::DeviceError(byte)),
            }
        }
        Err(I8042Error::ResendLimit)
    }

    pub fn send_device_command_with_arg(&self, command: u8, arg: u8) -> Result<(), I8042Error> {
        self.send_device_command(command)?;
        let mut attempt = 0usize;
        while attempt < RETRIES {
            self.write_data(arg)?;
            match self.read_data()? {
                PS2_ACK => return Ok(()),
                PS2_RESEND => attempt += 1,
                byte => return Err(I8042Error::DeviceError(byte)),
            }
        }
        Err(I8042Error::ResendLimit)
    }

    pub fn wait_for_bat(&self) -> Result<(), I8042Error> {
        let byte = self.read_data()?;
        if byte == PS2_BAT_OK {
            Ok(())
        } else {
            Err(I8042Error::DeviceError(byte))
        }
    }

    pub fn flush_output(&self) {
        let mut count = 0usize;
        while count < FLUSH_LIMIT {
            let status = unsafe { inb(STATUS_PORT) };
            if status & STATUS_OUTPUT_FULL == 0 {
                break;
            }
            let _ = unsafe { inb(DATA_PORT) };
            count += 1;
        }
    }

    fn wait_input_empty(&self) -> Result<(), I8042Error> {
        let mut wait = 0usize;
        while wait < WAIT_LIMIT {
            if unsafe { inb(STATUS_PORT) } & STATUS_INPUT_FULL == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
            wait += 1;
        }
        Err(I8042Error::Timeout)
    }

    fn wait_output_full(&self) -> Result<(), I8042Error> {
        let mut wait = 0usize;
        while wait < WAIT_LIMIT {
            if unsafe { inb(STATUS_PORT) } & STATUS_OUTPUT_FULL != 0 {
                return Ok(());
            }
            core::hint::spin_loop();
            wait += 1;
        }
        Err(I8042Error::Timeout)
    }

    pub const fn status_aux_data(status: u8) -> bool {
        status & STATUS_AUX_DATA != 0
    }
}
