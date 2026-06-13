//! x86_64 hardware device registration glue.

use crate::arch::x86_64::boot::BootInfo;
use crate::kernel::device::{DeviceError, DeviceManager};

#[cfg(feature = "hw-laptop-hotkeys")]
use super::acpi_ec::ACPI_EC_HOTKEY_DRIVER;
#[cfg(feature = "hw-ahci")]
use super::ahci::AHCI_SATA0_DRIVER;
use super::limine_block::LIMINE_MODULE_BLOCK_DRIVER;
#[cfg(feature = "hw-ps2-keyboard")]
use super::ps2_keyboard::PS2_KEYBOARD_DRIVER;
use super::uart16550::UART16550_COM1_DRIVER;
#[cfg(feature = "hw-usb-hid")]
use super::xhci_keyboard::USB_HID_KEYBOARD_DRIVER;

pub fn register_real_drivers<const MAX: usize>(
    manager: &mut DeviceManager<MAX>,
    boot_info: Option<&BootInfo>,
) -> Result<(), DeviceError> {
    manager.register_driver(&UART16550_COM1_DRIVER)?;
    #[cfg(feature = "hw-ps2-keyboard")]
    manager.register_driver(&PS2_KEYBOARD_DRIVER)?;
    #[cfg(feature = "hw-usb-hid")]
    manager.register_driver(&USB_HID_KEYBOARD_DRIVER)?;
    #[cfg(feature = "hw-laptop-hotkeys")]
    manager.register_driver(&ACPI_EC_HOTKEY_DRIVER)?;

    if let Some(boot_info) = boot_info {
        if LIMINE_MODULE_BLOCK_DRIVER.configure_from_modules(boot_info.modules) {
            manager.register_driver(&LIMINE_MODULE_BLOCK_DRIVER)?;
        }
    }

    #[cfg(feature = "hw-ahci")]
    if super::ahci::lookup_by_name("sata0").is_some() {
        manager.register_driver(&AHCI_SATA0_DRIVER)?;
    }

    Ok(())
}

pub const fn limine_module_block() -> &'static super::limine_block::LimineModuleBlockDriver {
    &LIMINE_MODULE_BLOCK_DRIVER
}
