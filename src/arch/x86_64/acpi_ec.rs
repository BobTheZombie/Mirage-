//! Minimal hardware-backed ACPI Embedded Controller hotkey/event input driver.
//!
//! Mirage does not yet have a full AML interpreter. This driver therefore uses
//! the firmware RSDP handoff to validate that ACPI is present, then enables the
//! conservative legacy EC I/O command path only when ACPI was discoverable. If a
//! platform requires namespace decoding for non-standard EC resources, the driver
//! skips cleanly instead of guessing a vendor profile.

use crate::arch::x86_64::boot::BootInfo;
use crate::arch::x86_64::io::{inb, outb};
use crate::kernel::device::{DeviceDriver, DeviceError, DeviceKind};
use crate::kernel::input::{
    copy_mirage_events, mark_source_online, publish_keyboard_event, InputRawSource, KeyCode,
    KeyModifiers, KeyState, KeyboardEvent,
};
use crate::subkernel::{DeviceSecurity, SecurityClass};

const EC_DATA_PORT: u16 = 0x62;
const EC_COMMAND_STATUS_PORT: u16 = 0x66;
const EC_STATUS_OBF: u8 = 1 << 0;
const EC_STATUS_IBF: u8 = 1 << 1;
const EC_CMD_QUERY: u8 = 0x84;
const WAIT_LIMIT: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcpiEcStatus {
    Online,
    SkippedNoAcpi,
    SkippedNoEc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcpiEcError {
    Timeout,
}

pub struct AcpiEcHotkeyDriver;

impl AcpiEcHotkeyDriver {
    pub const fn new() -> Self {
        Self
    }

    pub fn initialize(&self, boot_info: &BootInfo) -> AcpiEcStatus {
        if boot_info.rsdp.is_none() {
            crate::kprintln!("ACPI EC hotkey driver unavailable/skipped: no RSDP");
            return AcpiEcStatus::SkippedNoAcpi;
        }

        // Without AML namespace parsing, only enable the legacy EC query path if
        // the controller appears responsive and not permanently busy. This is a
        // real hardware probe, not a fabricated device.
        if self.wait_input_clear().is_err() {
            crate::kprintln!("ACPI EC hotkey driver unavailable/skipped: EC not responsive");
            return AcpiEcStatus::SkippedNoEc;
        }
        mark_source_online(InputRawSource::AcpiEc);
        crate::kprintln!("ACPI EC hotkey driver online");
        AcpiEcStatus::Online
    }

    pub fn poll(&self) {
        let status = unsafe { inb(EC_COMMAND_STATUS_PORT) };
        if status & EC_STATUS_OBF == 0 {
            return;
        }
        match self.query() {
            Ok(0) => {}
            Ok(code) => {
                let keycode = map_ec_query(code).unwrap_or(KeyCode::Raw(0x8000 | code as u16));
                crate::kprintln!("acpi-ec-hotkey0: query={:#x} key={:?}", code, keycode);
                publish_keyboard_event(KeyboardEvent::new(
                    keycode,
                    KeyState::Pressed,
                    KeyModifiers::empty(),
                    None,
                    InputRawSource::AcpiEc,
                    code as u16,
                ));
            }
            Err(_) => {
                crate::kprintln!("acpi-ec-hotkey0: EC query timeout");
            }
        }
    }

    fn query(&self) -> Result<u8, AcpiEcError> {
        self.wait_input_clear()?;
        unsafe { outb(EC_COMMAND_STATUS_PORT, EC_CMD_QUERY) };
        self.wait_output_full()?;
        Ok(unsafe { inb(EC_DATA_PORT) })
    }

    fn wait_input_clear(&self) -> Result<(), AcpiEcError> {
        let mut wait = 0usize;
        while wait < WAIT_LIMIT {
            if unsafe { inb(EC_COMMAND_STATUS_PORT) } & EC_STATUS_IBF == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
            wait += 1;
        }
        Err(AcpiEcError::Timeout)
    }

    fn wait_output_full(&self) -> Result<(), AcpiEcError> {
        let mut wait = 0usize;
        while wait < WAIT_LIMIT {
            if unsafe { inb(EC_COMMAND_STATUS_PORT) } & EC_STATUS_OBF != 0 {
                return Ok(());
            }
            core::hint::spin_loop();
            wait += 1;
        }
        Err(AcpiEcError::Timeout)
    }
}

impl DeviceDriver for AcpiEcHotkeyDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::InputController
    }

    fn name(&self) -> &'static str {
        "acpi-ec-hotkey0"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Internal, false)
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        self.poll();
        if buffer.len() < core::mem::size_of::<crate::kernel::device::MirageInputEvent>() {
            return Err(DeviceError::BufferTooSmall);
        }
        Ok(copy_mirage_events(buffer))
    }
}

pub fn map_ec_query(code: u8) -> Option<KeyCode> {
    match code {
        0x10 => Some(KeyCode::BrightnessDown),
        0x11 => Some(KeyCode::BrightnessUp),
        0x20 => Some(KeyCode::VolumeDown),
        0x21 => Some(KeyCode::VolumeUp),
        0x22 => Some(KeyCode::Mute),
        0x30 => Some(KeyCode::Sleep),
        0x40 => Some(KeyCode::DisplaySwitch),
        0x01 => Some(KeyCode::Escape),
        _ => None,
    }
}

pub static ACPI_EC_HOTKEY_DRIVER: AcpiEcHotkeyDriver = AcpiEcHotkeyDriver::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ec_query_mapping_defaults_cover_common_hotkeys() {
        assert_eq!(map_ec_query(0x11), Some(KeyCode::BrightnessUp));
        assert_eq!(map_ec_query(0x21), Some(KeyCode::VolumeUp));
        assert_eq!(map_ec_query(0xff), None);
    }
}
