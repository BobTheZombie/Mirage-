//! x86_64 hardware device registration glue.

use crate::arch::x86_64::boot::BootInfo;
use crate::kernel::device::{DeviceError, DeviceManager};

use super::limine_block::LIMINE_MODULE_BLOCK_DRIVER;
use super::ps2_keyboard::PS2_KEYBOARD_DRIVER;
use super::uart16550::UART16550_COM1_DRIVER;

pub fn register_real_drivers<const MAX: usize>(
    manager: &mut DeviceManager<MAX>,
    boot_info: Option<&BootInfo>,
) -> Result<(), DeviceError> {
    manager.register_driver(&UART16550_COM1_DRIVER)?;
    manager.register_driver(&PS2_KEYBOARD_DRIVER)?;

    if let Some(boot_info) = boot_info {
        if LIMINE_MODULE_BLOCK_DRIVER.configure_from_modules(boot_info.modules) {
            manager.register_driver(&LIMINE_MODULE_BLOCK_DRIVER)?;
        }
    }

    Ok(())
}

pub const fn limine_module_block() -> &'static super::limine_block::LimineModuleBlockDriver {
    &LIMINE_MODULE_BLOCK_DRIVER
}
