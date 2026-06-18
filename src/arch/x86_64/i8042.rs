//! Real x86_64 i8042 PS/2 controller bring-up for built-in laptop keyboards.

use crate::arch::x86_64::io::{inb, io_wait, outb};

pub const DATA_PORT: u16 = 0x60;
pub const STATUS_PORT: u16 = 0x64;
pub const COMMAND_PORT: u16 = 0x64;

const STATUS_OUTPUT_FULL: u8 = 0x01;
const STATUS_INPUT_FULL: u8 = 0x02;
const STATUS_AUX_DATA: u8 = 0x20;
const STATUS_TIMEOUT_ERROR: u8 = 0x40;
const STATUS_PARITY_ERROR: u8 = 0x80;

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
    ControllerTimeout,
    ControllerParity,
    SelfTestFailed(u8),
    PortTestFailed(u8),
    DeviceError(u8),
    ResendLimit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum I8042ControllerState {
    Absent,
    Detected,
    Started,
    Ready,
    Failed(I8042Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct I8042Status {
    pub output_full: bool,
    pub input_full: bool,
    pub system_flag: bool,
    pub command_data: bool,
    pub aux_data: bool,
    pub timeout_error: bool,
    pub parity_error: bool,
    pub raw: u8,
}

impl I8042Status {
    pub const fn from_raw(raw: u8) -> Self {
        Self {
            output_full: raw & STATUS_OUTPUT_FULL != 0,
            input_full: raw & STATUS_INPUT_FULL != 0,
            system_flag: raw & 0x04 != 0,
            command_data: raw & 0x08 != 0,
            aux_data: raw & STATUS_AUX_DATA != 0,
            timeout_error: raw & STATUS_TIMEOUT_ERROR != 0,
            parity_error: raw & STATUS_PARITY_ERROR != 0,
            raw,
        }
    }
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
        crate::kprintln!("[i8042 01] controller probe enter");
        self.write_command(CMD_DISABLE_FIRST)?;
        let _ = self.write_command(CMD_DISABLE_SECOND);
        self.flush_output();

        let mut config = self.read_config()?;
        crate::kprintln!("[i8042 02] config byte read");
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
            crate::kprintln!("[i8042 03] keyboard port test failed");
            return Err(I8042Error::PortTestFailed(first_test));
        }
        crate::kprintln!("[i8042 03] keyboard port test ok");

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
        crate::kprintln!("[i8042 04] keyboard port enabled");
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
        // SAFETY: reading the i8042 status port is a side-effect-free byte I/O read owned by this lower-kernel driver.
        unsafe { inb(STATUS_PORT) }
    }

    pub fn parsed_status(&self) -> I8042Status {
        I8042Status::from_raw(self.status())
    }

    pub fn read_data(&self) -> Result<u8, I8042Error> {
        self.wait_output_full()?;
        let status = self.parsed_status();
        if status.timeout_error {
            return Err(I8042Error::ControllerTimeout);
        }
        if status.parity_error {
            return Err(I8042Error::ControllerParity);
        }
        // SAFETY: the output-buffer-full bit was observed, and the i8042 data port is owned by this driver.
        Ok(unsafe { inb(DATA_PORT) })
    }

    pub fn write_data(&self, value: u8) -> Result<(), I8042Error> {
        self.wait_input_empty()?;
        // SAFETY: the input-buffer-empty bit was observed, and the i8042 data port is owned by this driver.
        unsafe { outb(DATA_PORT, value) };
        io_wait();
        Ok(())
    }

    pub fn write_command(&self, command: u8) -> Result<(), I8042Error> {
        self.wait_input_empty()?;
        // SAFETY: the input-buffer-empty bit was observed, and the i8042 command port is owned by this driver.
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

    pub fn wait_input_empty_timeout(&self, timeout: usize) -> Result<(), I8042Error> {
        self.wait_input_empty_for(timeout)
    }

    pub fn wait_output_full_timeout(&self, timeout: usize) -> Result<(), I8042Error> {
        self.wait_output_full_for(timeout)
    }

    pub fn read_data_timeout(&self, timeout: usize) -> Result<u8, I8042Error> {
        self.wait_output_full_for(timeout)?;
        let status = self.parsed_status();
        if status.timeout_error {
            return Err(I8042Error::ControllerTimeout);
        }
        if status.parity_error {
            return Err(I8042Error::ControllerParity);
        }
        // SAFETY: the output-buffer-full bit was observed, and the i8042 data port is owned by this driver.
        Ok(unsafe { inb(DATA_PORT) })
    }

    pub fn write_data_timeout(&self, value: u8, timeout: usize) -> Result<(), I8042Error> {
        self.wait_input_empty_for(timeout)?;
        // SAFETY: the input-buffer-empty bit was observed, and the i8042 data port is owned by this driver.
        unsafe { outb(DATA_PORT, value) };
        io_wait();
        Ok(())
    }

    pub fn write_command_timeout(&self, command: u8, timeout: usize) -> Result<(), I8042Error> {
        self.wait_input_empty_for(timeout)?;
        // SAFETY: the input-buffer-empty bit was observed, and the i8042 command port is owned by this driver.
        unsafe { outb(COMMAND_PORT, command) };
        io_wait();
        Ok(())
    }

    fn wait_input_empty(&self) -> Result<(), I8042Error> {
        self.wait_input_empty_for(WAIT_LIMIT)
    }

    fn wait_input_empty_for(&self, limit: usize) -> Result<(), I8042Error> {
        let mut wait = 0usize;
        while wait < limit {
            if unsafe { inb(STATUS_PORT) } & STATUS_INPUT_FULL == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
            wait += 1;
        }
        Err(I8042Error::Timeout)
    }

    fn wait_output_full(&self) -> Result<(), I8042Error> {
        self.wait_output_full_for(WAIT_LIMIT)
    }

    fn wait_output_full_for(&self, limit: usize) -> Result<(), I8042Error> {
        let mut wait = 0usize;
        while wait < limit {
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
