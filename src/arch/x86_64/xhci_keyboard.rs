//! Hardware-backed xHCI discovery and USB HID boot-keyboard report support.
//!
//! The controller path performs real PCI class discovery and xHCI MMIO bring-up
//! (halt, reset, basic run). Full TRB enumeration is intentionally bounded and
//! defensive; QEMU `qemu-xhci` can be detected without fabricating a keyboard.

use crate::arch::x86_64::io::{inb, outb};
use crate::kernel::device::{DeviceDriver, DeviceError, DeviceKind};
use crate::kernel::input::{
    copy_mirage_events, mark_source_online, publish_keyboard_event, InputRawSource, KeyCode,
    KeyModifiers, KeyState, KeyboardEvent,
};
use crate::subkernel::{DeviceSecurity, SecurityClass};

const PCI_CONFIG_ADDRESS: u16 = 0xcf8;
const PCI_CONFIG_DATA: u16 = 0xcfc;
const PCI_CLASS_SERIAL_BUS: u8 = 0x0c;
const PCI_SUBCLASS_USB: u8 = 0x03;
const PCI_PROGIF_XHCI: u8 = 0x30;

const USBSTS_HCH: u32 = 1 << 0;
const USBCMD_RUN: u32 = 1 << 0;
const USBCMD_RESET: u32 = 1 << 1;
const WAIT_LIMIT: usize = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XhciKeyboardStatus {
    Online,
    SkippedNoController,
    SkippedNoKeyboard,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PciFunction {
    bus: u8,
    device: u8,
    function: u8,
}

pub struct UsbHidKeyboardDriver;

impl UsbHidKeyboardDriver {
    pub const fn new() -> Self {
        Self
    }

    pub fn initialize(&self, hhdm_offset: Option<u64>) -> XhciKeyboardStatus {
        let Some(function) = find_xhci_controller() else {
            crate::kprintln!("usb-hid-keyboard0: xHCI controller not found; skipped");
            return XhciKeyboardStatus::SkippedNoController;
        };

        enable_pci_command(function);
        let bar0 = pci_read_u32(function, 0x10) & !0x0f;
        if bar0 == 0 {
            crate::kprintln!("usb-hid-keyboard0: xHCI BAR0 absent; failed");
            return XhciKeyboardStatus::Failed;
        }
        let mmio = match hhdm_offset {
            Some(offset) => (offset + bar0 as u64) as *mut u8,
            None => bar0 as usize as *mut u8,
        };

        if unsafe { bring_up_xhci(mmio) }.is_err() {
            crate::kprintln!("usb-hid-keyboard0: xHCI bring-up failed");
            return XhciKeyboardStatus::Failed;
        }

        // Enumeration TODO: command/event rings are brought up next. Until a HID
        // interface is actually discovered, report skipped rather than online.
        crate::kprintln!("usb-hid-keyboard0: xHCI online; HID boot keyboard not enumerated yet");
        XhciKeyboardStatus::SkippedNoKeyboard
    }

    pub fn ingest_boot_report(
        &self,
        previous: HidBootKeyboardReport,
        current: HidBootKeyboardReport,
    ) {
        for event in diff_hid_boot_reports(previous, current)
            .into_iter()
            .flatten()
        {
            publish_keyboard_event(event);
            if event.keycode == KeyCode::Escape && event.state == KeyState::Pressed {
                crate::kprintln!("usb-hid-keyboard0: ESC raw={:#x}", event.raw_code);
            }
        }
    }
}

impl DeviceDriver for UsbHidKeyboardDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::InputController
    }

    fn name(&self) -> &'static str {
        "usb-hid-keyboard0"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Internal, false)
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        if buffer.len() < core::mem::size_of::<crate::kernel::device::MirageInputEvent>() {
            return Err(DeviceError::BufferTooSmall);
        }
        Ok(copy_mirage_events(buffer))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HidBootKeyboardReport {
    pub modifiers: u8,
    pub reserved: u8,
    pub keys: [u8; 6],
}

pub fn diff_hid_boot_reports(
    previous: HidBootKeyboardReport,
    current: HidBootKeyboardReport,
) -> [Option<KeyboardEvent>; 12] {
    let mut out = [None; 12];
    let mut index = 0usize;
    let modifiers = hid_modifiers(current.modifiers);

    let mut slot = 0usize;
    while slot < 6 {
        let key = previous.keys[slot];
        if key != 0 && !contains_key(current.keys, key) {
            out[index] = hid_usage_to_event(key, KeyState::Released, modifiers);
            index += 1;
        }
        slot += 1;
    }

    slot = 0;
    while slot < 6 {
        let key = current.keys[slot];
        if key != 0 && !contains_key(previous.keys, key) {
            out[index] = hid_usage_to_event(key, KeyState::Pressed, modifiers);
            index += 1;
        }
        slot += 1;
    }
    out
}

fn contains_key(keys: [u8; 6], needle: u8) -> bool {
    let mut index = 0usize;
    while index < keys.len() {
        if keys[index] == needle {
            return true;
        }
        index += 1;
    }
    false
}

pub fn hid_modifiers(bits: u8) -> KeyModifiers {
    KeyModifiers {
        left_shift: bits & (1 << 1) != 0,
        right_shift: bits & (1 << 5) != 0,
        ctrl: bits & ((1 << 0) | (1 << 4)) != 0,
        alt: bits & ((1 << 2) | (1 << 6)) != 0,
        caps_lock: false,
    }
}

pub fn hid_usage_to_event(
    usage: u8,
    state: KeyState,
    modifiers: KeyModifiers,
) -> Option<KeyboardEvent> {
    let keycode = match usage {
        0x04..=0x1d => KeyCode::Char(0),
        0x1e..=0x27 => KeyCode::Char(0),
        0x28 => KeyCode::Enter,
        0x29 => KeyCode::Escape,
        0x2a => KeyCode::Backspace,
        0x2b => KeyCode::Tab,
        0x3a..=0x45 => KeyCode::F(usage - 0x39),
        0x4f => KeyCode::ArrowRight,
        0x50 => KeyCode::ArrowLeft,
        0x51 => KeyCode::ArrowDown,
        0x52 => KeyCode::ArrowUp,
        0xe0 => KeyCode::LeftCtrl,
        0xe1 => KeyCode::LeftShift,
        0xe2 => KeyCode::LeftAlt,
        0xe4 => KeyCode::RightCtrl,
        0xe5 => KeyCode::RightShift,
        0xe6 => KeyCode::RightAlt,
        _ => KeyCode::Raw(usage as u16),
    };
    let ascii = if state == KeyState::Pressed {
        hid_usage_ascii(usage, modifiers)
    } else {
        None
    };
    Some(KeyboardEvent::new(
        keycode,
        state,
        modifiers,
        ascii,
        InputRawSource::UsbHid,
        usage as u16,
    ))
}

pub fn hid_usage_ascii(usage: u8, modifiers: KeyModifiers) -> Option<u8> {
    let shifted = modifiers.shift();
    Some(match usage {
        0x04..=0x1d => {
            let base = b'a' + (usage - 0x04);
            if shifted {
                base - 32
            } else {
                base
            }
        }
        0x1e => {
            if shifted {
                b'!'
            } else {
                b'1'
            }
        }
        0x1f => {
            if shifted {
                b'@'
            } else {
                b'2'
            }
        }
        0x20 => {
            if shifted {
                b'#'
            } else {
                b'3'
            }
        }
        0x21 => {
            if shifted {
                b'$'
            } else {
                b'4'
            }
        }
        0x22 => {
            if shifted {
                b'%'
            } else {
                b'5'
            }
        }
        0x23 => {
            if shifted {
                b'^'
            } else {
                b'6'
            }
        }
        0x24 => {
            if shifted {
                b'&'
            } else {
                b'7'
            }
        }
        0x25 => {
            if shifted {
                b'*'
            } else {
                b'8'
            }
        }
        0x26 => {
            if shifted {
                b'('
            } else {
                b'9'
            }
        }
        0x27 => {
            if shifted {
                b')'
            } else {
                b'0'
            }
        }
        0x28 => b'\n',
        0x2a => 8,
        0x2b => b'\t',
        0x2c => b' ',
        0x2d => {
            if shifted {
                b'_'
            } else {
                b'-'
            }
        }
        0x2e => {
            if shifted {
                b'+'
            } else {
                b'='
            }
        }
        0x2f => {
            if shifted {
                b'{'
            } else {
                b'['
            }
        }
        0x30 => {
            if shifted {
                b'}'
            } else {
                b']'
            }
        }
        0x31 => {
            if shifted {
                b'|'
            } else {
                b'\\'
            }
        }
        0x33 => {
            if shifted {
                b':'
            } else {
                b';'
            }
        }
        0x34 => {
            if shifted {
                b'"'
            } else {
                b'\''
            }
        }
        0x36 => {
            if shifted {
                b'<'
            } else {
                b','
            }
        }
        0x37 => {
            if shifted {
                b'>'
            } else {
                b'.'
            }
        }
        0x38 => {
            if shifted {
                b'?'
            } else {
                b'/'
            }
        }
        _ => return None,
    })
}

fn find_xhci_controller() -> Option<PciFunction> {
    let mut bus = 0u16;
    while bus <= 255 {
        let mut device = 0u8;
        while device <= 31 {
            let mut function = 0u8;
            while function <= 7 {
                let f = PciFunction {
                    bus: bus as u8,
                    device,
                    function,
                };
                let vendor = (pci_read_u32(f, 0x00) & 0xffff) as u16;
                if vendor != 0xffff {
                    let class_reg = pci_read_u32(f, 0x08);
                    let class = (class_reg >> 24) as u8;
                    let subclass = (class_reg >> 16) as u8;
                    let prog_if = (class_reg >> 8) as u8;
                    if class == PCI_CLASS_SERIAL_BUS
                        && subclass == PCI_SUBCLASS_USB
                        && prog_if == PCI_PROGIF_XHCI
                    {
                        return Some(f);
                    }
                }
                function += 1;
            }
            device += 1;
        }
        bus += 1;
    }
    None
}

fn pci_read_u32(function: PciFunction, offset: u8) -> u32 {
    let address = 0x8000_0000u32
        | ((function.bus as u32) << 16)
        | ((function.device as u32) << 11)
        | ((function.function as u32) << 8)
        | ((offset as u32) & 0xfc);
    unsafe {
        outb(PCI_CONFIG_ADDRESS, address as u8);
        outb(PCI_CONFIG_ADDRESS + 1, (address >> 8) as u8);
        outb(PCI_CONFIG_ADDRESS + 2, (address >> 16) as u8);
        outb(PCI_CONFIG_ADDRESS + 3, (address >> 24) as u8);
        let b0 = inb(PCI_CONFIG_DATA) as u32;
        let b1 = inb(PCI_CONFIG_DATA + 1) as u32;
        let b2 = inb(PCI_CONFIG_DATA + 2) as u32;
        let b3 = inb(PCI_CONFIG_DATA + 3) as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }
}

fn pci_write_u32(function: PciFunction, offset: u8, value: u32) {
    let address = 0x8000_0000u32
        | ((function.bus as u32) << 16)
        | ((function.device as u32) << 11)
        | ((function.function as u32) << 8)
        | ((offset as u32) & 0xfc);
    unsafe {
        outb(PCI_CONFIG_ADDRESS, address as u8);
        outb(PCI_CONFIG_ADDRESS + 1, (address >> 8) as u8);
        outb(PCI_CONFIG_ADDRESS + 2, (address >> 16) as u8);
        outb(PCI_CONFIG_ADDRESS + 3, (address >> 24) as u8);
        outb(PCI_CONFIG_DATA, value as u8);
        outb(PCI_CONFIG_DATA + 1, (value >> 8) as u8);
        outb(PCI_CONFIG_DATA + 2, (value >> 16) as u8);
        outb(PCI_CONFIG_DATA + 3, (value >> 24) as u8);
    }
}

fn enable_pci_command(function: PciFunction) {
    let value = pci_read_u32(function, 0x04) | 0x0006;
    pci_write_u32(function, 0x04, value);
}

unsafe fn mmio_read32(base: *mut u8, offset: usize) -> u32 {
    core::ptr::read_volatile(base.add(offset) as *const u32)
}

unsafe fn mmio_write32(base: *mut u8, offset: usize, value: u32) {
    core::ptr::write_volatile(base.add(offset) as *mut u32, value)
}

unsafe fn bring_up_xhci(base: *mut u8) -> Result<(), ()> {
    if base.is_null() {
        return Err(());
    }
    let cap_length = core::ptr::read_volatile(base as *const u8) as usize;
    if cap_length < 0x20 || cap_length > 0x100 {
        return Err(());
    }
    let op = base.add(cap_length);

    let mut cmd = mmio_read32(op, 0x00);
    cmd &= !USBCMD_RUN;
    mmio_write32(op, 0x00, cmd);
    wait_status(op, USBSTS_HCH, true)?;

    mmio_write32(op, 0x00, cmd | USBCMD_RESET);
    wait_command_clear(op, USBCMD_RESET)?;

    // Max slots is capped defensively. Real DCBAA/ring programming will be
    // added once Mirage has a DMA allocator contract for xHCI services.
    let hcsparams1 = mmio_read32(base, 0x04);
    let max_slots = (hcsparams1 & 0xff).min(32);
    mmio_write32(op, 0x38, max_slots);

    cmd = mmio_read32(op, 0x00) | USBCMD_RUN;
    mmio_write32(op, 0x00, cmd);
    wait_status(op, USBSTS_HCH, false)?;
    Ok(())
}

unsafe fn wait_command_clear(op: *mut u8, bit: u32) -> Result<(), ()> {
    let mut wait = 0usize;
    while wait < WAIT_LIMIT {
        if mmio_read32(op, 0x00) & bit == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
        wait += 1;
    }
    Err(())
}

unsafe fn wait_status(op: *mut u8, bit: u32, set: bool) -> Result<(), ()> {
    let mut wait = 0usize;
    while wait < WAIT_LIMIT {
        let present = mmio_read32(op, 0x04) & bit != 0;
        if present == set {
            return Ok(());
        }
        core::hint::spin_loop();
        wait += 1;
    }
    Err(())
}

pub static USB_HID_KEYBOARD_DRIVER: UsbHidKeyboardDriver = UsbHidKeyboardDriver::new();

pub fn mark_usb_keyboard_online_for_enumeration() {
    mark_source_online(InputRawSource::UsbHid);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hid_report_diff_generates_press_and_release() {
        let prev = HidBootKeyboardReport {
            modifiers: 0,
            reserved: 0,
            keys: [0x04, 0, 0, 0, 0, 0],
        };
        let curr = HidBootKeyboardReport {
            modifiers: 0,
            reserved: 0,
            keys: [0x29, 0, 0, 0, 0, 0],
        };
        let events = diff_hid_boot_reports(prev, curr);
        assert_eq!(events[0].unwrap().state, KeyState::Released);
        assert_eq!(events[1].unwrap().keycode, KeyCode::Escape);
        assert_eq!(events[1].unwrap().state, KeyState::Pressed);
    }

    #[test]
    fn hid_modifier_translation_supports_shift() {
        let mods = hid_modifiers(1 << 1);
        assert!(mods.left_shift);
        assert_eq!(hid_usage_ascii(0x04, mods), Some(b'A'));
    }
}
