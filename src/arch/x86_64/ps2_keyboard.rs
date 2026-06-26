//! Hardware-backed PS/2 keyboard driver using the x86_64 i8042 controller.

use core::cmp::min;

use crate::arch::x86_64::i8042::{I8042Controller, I8042ControllerState, I8042Error};
use crate::kernel::device::{
    copy_input_event_to_bytes, DeviceDriver, DeviceError, DeviceKind, MirageInputEvent,
};
use crate::kernel::input::{
    mark_source_online, publish_keyboard_event, try_publish_keyboard_event, InputRawSource,
    KeyCode, KeyModifiers, KeyState, KeyboardEvent,
};
use crate::kernel::sync::SpinLock;
use crate::subkernel::{DeviceSecurity, SecurityClass};

const MAX_POLL_BYTES: usize = 32;
const START_DRAIN_BYTES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ps2ScanSet {
    Set1Translated,
    Set2,
}

#[derive(Clone, Copy)]
struct DriverState {
    initialized: bool,
    online: bool,
    irq_mode: bool,
    controller_state: I8042ControllerState,
    scan_set: Ps2ScanSet,
    decoder: Ps2Decoder,
    events: [MirageInputEvent; Ps2KeyboardDriver::QUEUE_CAPACITY],
    head: usize,
    len: usize,
    bytes_received: u64,
    events_received: u64,
    decode_errors: u64,
    irq_drops: u64,
    first_event_logged: bool,
    last_event: Option<KeyboardEvent>,
}

impl DriverState {
    const fn new() -> Self {
        Self {
            initialized: false,
            online: false,
            irq_mode: false,
            controller_state: I8042ControllerState::Absent,
            scan_set: Ps2ScanSet::Set1Translated,
            decoder: Ps2Decoder::new(Ps2ScanSet::Set1Translated),
            events: [MirageInputEvent {
                event_type: 0,
                code: 0,
                value: 0,
                timestamp_ns: 0,
            }; Ps2KeyboardDriver::QUEUE_CAPACITY],
            head: 0,
            len: 0,
            bytes_received: 0,
            events_received: 0,
            decode_errors: 0,
            irq_drops: 0,
            first_event_logged: false,
            last_event: None,
        }
    }

    fn push(&mut self, event: MirageInputEvent) {
        if self.len == self.events.len() {
            self.head = (self.head + 1) % self.events.len();
            self.len -= 1;
        }
        let tail = (self.head + self.len) % self.events.len();
        self.events[tail] = event;
        self.len += 1;
    }

    fn pop(&mut self) -> Option<MirageInputEvent> {
        if self.len == 0 {
            return None;
        }
        let event = self.events[self.head];
        self.head = (self.head + 1) % self.events.len();
        self.len -= 1;
        Some(event)
    }
}

pub struct Ps2KeyboardDriver {
    controller: I8042Controller,
    state: SpinLock<DriverState>,
}

impl Ps2KeyboardDriver {
    pub const QUEUE_CAPACITY: usize = 64;

    pub const fn new() -> Self {
        Self {
            controller: I8042Controller::new(),
            state: SpinLock::new(DriverState::new()),
        }
    }

    pub fn initialize(&self, irq_mode: bool) -> Result<(), I8042Error> {
        crate::kprintln!("I8042             [ DETECTED ]");
        let init = self.controller.initialize(irq_mode, true)?;
        crate::kprintln!("I8042             [ STARTED ]");

        let mut scan_set = if init.translated {
            Ps2ScanSet::Set1Translated
        } else {
            Ps2ScanSet::Set2
        };

        // The PS/2 command path is best-effort during early boot.  Controller
        // failure is reported by returning Err above, but keyboard command
        // timeouts are not allowed to block or abort the post-kernel boot path:
        // QEMU, VirtualBox, firmware, and real laptops differ in when BAT and
        // identify bytes are delivered.  If scan bytes arrive later, the
        // polling/IRQ paths will decode them in degraded mode.
        if let Err(error) = self.controller.send_device_command(0xf5) {
            crate::kprintln!(
                "PS/2 KEYBOARD     [ DEGRADED: disable scanning {:?} ]",
                error
            );
        }
        if let Err(error) = self.controller.send_device_command(0xff) {
            crate::kprintln!("PS/2 KEYBOARD     [ DEGRADED: reset {:?} ]", error);
        } else if let Err(error) = self.controller.wait_for_bat() {
            crate::kprintln!("PS/2 KEYBOARD     [ DEGRADED: BAT {:?} ]", error);
        }
        if let Err(error) = self.controller.send_device_command(0xf2) {
            crate::kprintln!("PS/2 KEYBOARD     [ DEGRADED: identify {:?} ]", error);
        }
        if !init.translated {
            match self.controller.send_device_command_with_arg(0xf0, 0x02) {
                Ok(()) => scan_set = Ps2ScanSet::Set2,
                Err(error) => {
                    crate::kprintln!("PS/2 KEYBOARD     [ DEGRADED: set scancode {:?} ]", error);
                    scan_set = Ps2ScanSet::Set2;
                }
            }
        }
        if let Err(error) = self.controller.send_device_command(0xf4) {
            crate::kprintln!(
                "PS/2 KEYBOARD     [ DEGRADED: enable scanning {:?} ]",
                error
            );
        }

        let mut state = self.state.lock();
        state.initialized = true;
        state.irq_mode = irq_mode;
        state.controller_state = I8042ControllerState::Ready;
        state.scan_set = scan_set;
        state.decoder = Ps2Decoder::new(scan_set);
        crate::kprintln!(
            "PS/2 KEYBOARD     [ STARTED: {} mode ]",
            if irq_mode { "irq" } else { "polling" }
        );
        // Drain at most a few pending controller bytes left by reset/identify.
        // This is not a wait for user input; it is a bounded cleanup pass so
        // stale ACK/BAT/ID bytes cannot poison the first normal poll.
        let _ = self.drain_keyboard_events(START_DRAIN_BYTES);
        crate::kprintln!("PS/2 KEYBOARD     [ OK ]");
        Ok(())
    }

    pub fn poll_keyboard_once(&self) -> Result<Option<KeyboardEvent>, I8042Error> {
        let status = self.controller.status();
        if status & 0x01 == 0 {
            return Ok(None);
        }
        let Some(scancode) = self.controller.read_data_nonblocking()? else {
            return Ok(None);
        };
        if I8042Controller::status_aux_data(status) {
            return Ok(None);
        }

        let mut state = self.state.lock();
        state.bytes_received = state.bytes_received.saturating_add(1);
        let events = state.decoder.feed(scancode);
        let mut first_event = None;
        for event in events.into_iter().flatten() {
            if first_event.is_none() {
                first_event = Some(event);
            }
            let abi = event.to_mirage_input_event();
            publish_keyboard_event(event);
            state.push(abi);
            state.events_received = state.events_received.saturating_add(1);
            state.last_event = Some(event);
            if !state.first_event_logged {
                state.first_event_logged = true;
                state.online = true;
                mark_source_online(InputRawSource::Ps2);
                crate::kprintln!("[kbd 06] first key event received");
            }
        }
        Ok(first_event)
    }

    pub fn drain_keyboard_events(&self, max_events: usize) -> Result<usize, I8042Error> {
        let limit = min(max_events, MAX_POLL_BYTES);
        let mut drained = 0usize;
        while drained < limit {
            if self.poll_keyboard_once()?.is_none() {
                break;
            }
            drained += 1;
        }
        Ok(drained)
    }

    pub fn poll_hardware(&self) {
        let _ = self.drain_keyboard_events(MAX_POLL_BYTES);
    }
    pub fn handle_irq1(&self) {
        let mut drained = 0usize;
        while drained < MAX_POLL_BYTES {
            let status = self.controller.status();
            if status & 0x01 == 0 {
                break;
            }
            let Ok(Some(scancode)) = self.controller.read_data_nonblocking() else {
                break;
            };
            if !I8042Controller::status_aux_data(status) {
                let Some(mut state) = self.state.try_lock() else {
                    // Do not spin in IRQ context.  Dropping one raw byte is
                    // safer than deadlocking the boot CPU while a normal path
                    // owns the decoder state.
                    break;
                };
                state.bytes_received = state.bytes_received.saturating_add(1);
                let events = state.decoder.feed(scancode);
                for event in events.into_iter().flatten() {
                    let abi = event.to_mirage_input_event();
                    if !try_publish_keyboard_event(event) {
                        state.irq_drops = state.irq_drops.saturating_add(1);
                    }
                    state.push(abi);
                    state.events_received = state.events_received.saturating_add(1);
                    state.last_event = Some(event);
                    if !state.first_event_logged {
                        state.first_event_logged = true;
                        state.online = true;
                        mark_source_online(InputRawSource::Ps2);
                    }
                }
            }
            drained += 1;
        }
    }

    pub fn status_snapshot(&self) -> Ps2KeyboardSnapshot {
        let state = self.state.lock();
        Ps2KeyboardSnapshot {
            initialized: state.initialized,
            online: state.online,
            irq_mode: state.irq_mode,
            controller_state: state.controller_state,
            scan_set: state.scan_set,
            bytes_received: state.bytes_received,
            events_received: state.events_received,
            decode_errors: state.decode_errors,
            irq_drops: state.irq_drops,
            queue_overflows: crate::kernel::input::input_queue_overflows(),
            last_event: state.last_event,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ps2KeyboardSnapshot {
    pub initialized: bool,
    pub online: bool,
    pub irq_mode: bool,
    pub controller_state: I8042ControllerState,
    pub scan_set: Ps2ScanSet,
    pub bytes_received: u64,
    pub events_received: u64,
    pub decode_errors: u64,
    pub irq_drops: u64,
    pub queue_overflows: u64,
    pub last_event: Option<KeyboardEvent>,
}

impl DeviceDriver for Ps2KeyboardDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::InputController
    }

    fn name(&self) -> &'static str {
        "ps2-keyboard0"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Internal, false)
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        self.poll_hardware();
        let event_size = core::mem::size_of::<MirageInputEvent>();
        if buffer.len() < event_size {
            return Err(DeviceError::BufferTooSmall);
        }

        let event_capacity = buffer.len() / event_size;
        let mut written = 0usize;
        let mut state = self.state.lock();
        while written < min(event_capacity, Self::QUEUE_CAPACITY) {
            let Some(event) = state.pop() else { break };
            copy_input_event_to_bytes(&event, &mut buffer[written * event_size..][..event_size])?;
            written += 1;
        }
        Ok(written * event_size)
    }
}

#[derive(Clone, Copy)]
pub struct Ps2Decoder {
    scan_set: Ps2ScanSet,
    extended: bool,
    break_pending: bool,
    modifiers: KeyModifiers,
    key_down: [bool; 512],
}

impl Ps2Decoder {
    pub const fn new(scan_set: Ps2ScanSet) -> Self {
        Self {
            scan_set,
            extended: false,
            break_pending: false,
            modifiers: KeyModifiers::empty(),
            key_down: [false; 512],
        }
    }

    pub fn feed(&mut self, byte: u8) -> [Option<KeyboardEvent>; 2] {
        match self.scan_set {
            Ps2ScanSet::Set1Translated => self.feed_set1(byte),
            Ps2ScanSet::Set2 => self.feed_set2(byte),
        }
    }

    fn feed_set1(&mut self, byte: u8) -> [Option<KeyboardEvent>; 2] {
        if byte == 0xe0 {
            self.extended = true;
            return [None, None];
        }
        if byte == 0xe1 {
            self.extended = false;
            return [None, None];
        }
        let released = byte & 0x80 != 0;
        let raw = (byte & 0x7f) as u16 | if self.extended { 0x0100 } else { 0 };
        let event = self
            .make_event(raw, !released)
            .map(Self::update_ascii_with_raw);
        self.extended = false;
        [event, None]
    }

    fn feed_set2(&mut self, byte: u8) -> [Option<KeyboardEvent>; 2] {
        match byte {
            0xe0 => {
                self.extended = true;
                [None, None]
            }
            0xf0 => {
                self.break_pending = true;
                [None, None]
            }
            0xe1 => {
                self.extended = false;
                self.break_pending = false;
                [None, None]
            }
            _ => {
                let raw =
                    set2_to_set1(byte, self.extended) | if self.extended { 0x0100 } else { 0 };
                let pressed = !self.break_pending;
                self.extended = false;
                self.break_pending = false;
                [
                    self.make_event(raw, pressed)
                        .map(Self::update_ascii_with_raw),
                    None,
                ]
            }
        }
    }

    fn make_event(&mut self, raw: u16, pressed: bool) -> Option<KeyboardEvent> {
        let keycode = map_set1_raw(raw)?;
        match keycode {
            KeyCode::LeftShift => self.modifiers.left_shift = pressed,
            KeyCode::RightShift => self.modifiers.right_shift = pressed,
            KeyCode::LeftCtrl | KeyCode::RightCtrl => self.modifiers.ctrl = pressed,
            KeyCode::LeftAlt | KeyCode::RightAlt => self.modifiers.alt = pressed,
            KeyCode::CapsLock if pressed => self.modifiers.caps_lock = !self.modifiers.caps_lock,
            KeyCode::NumLock if pressed => self.modifiers.num_lock = !self.modifiers.num_lock,
            KeyCode::ScrollLock if pressed => {
                self.modifiers.scroll_lock = !self.modifiers.scroll_lock
            }
            KeyCode::Meta => self.modifiers.meta = pressed,
            _ => {}
        }
        let idx = (raw as usize) & 0x01ff;
        let was_down = self.key_down[idx];
        if pressed {
            self.key_down[idx] = true;
        } else {
            self.key_down[idx] = false;
        }
        let ascii = if pressed && !was_down {
            ascii_for_key(keycode, self.modifiers)
        } else {
            None
        };
        Some(KeyboardEvent::new(
            keycode,
            if pressed {
                KeyState::Pressed
            } else {
                KeyState::Released
            },
            self.modifiers,
            ascii,
            InputRawSource::Ps2,
            raw,
        ))
    }
}

pub fn map_set1_raw(raw: u16) -> Option<KeyCode> {
    Some(match raw {
        0x01 => KeyCode::Escape,
        0x0e => KeyCode::Backspace,
        0x0f => KeyCode::Tab,
        0x1c => KeyCode::Enter,
        0x2a => KeyCode::LeftShift,
        0x36 => KeyCode::RightShift,
        0x1d => KeyCode::LeftCtrl,
        0x011d => KeyCode::RightCtrl,
        0x38 => KeyCode::LeftAlt,
        0x0138 => KeyCode::RightAlt,
        0x3a => KeyCode::CapsLock,
        0x45 => KeyCode::NumLock,
        0x46 => KeyCode::ScrollLock,
        0x015b | 0x015c => KeyCode::Meta,
        0x52 | 0x0152 => KeyCode::Insert,
        0x53 | 0x0153 => KeyCode::Delete,
        0x47 | 0x0147 => KeyCode::Home,
        0x4f | 0x014f => KeyCode::End,
        0x49 | 0x0149 => KeyCode::PageUp,
        0x51 | 0x0151 => KeyCode::PageDown,
        0x48 | 0x0148 => KeyCode::ArrowUp,
        0x50 | 0x0150 => KeyCode::ArrowDown,
        0x4b | 0x014b => KeyCode::ArrowLeft,
        0x4d | 0x014d => KeyCode::ArrowRight,
        0x3b..=0x44 => KeyCode::F((raw - 0x3a) as u8),
        0x57 => KeyCode::F(11),
        0x58 => KeyCode::F(12),
        0x02..=0x0d | 0x10..=0x19 | 0x1e..=0x28 | 0x2b | 0x2c..=0x35 | 0x39 => KeyCode::Char(0),
        _ => KeyCode::Raw(raw),
    })
}

pub fn ascii_for_key(keycode: KeyCode, _modifiers: KeyModifiers) -> Option<u8> {
    if keycode != KeyCode::Char(0) {
        return match keycode {
            KeyCode::Enter => Some(b'\n'),
            KeyCode::Backspace => Some(8),
            KeyCode::Tab => Some(b'\t'),
            _ => None,
        };
    }
    None
}

pub fn ascii_for_set1_raw(raw: u16, modifiers: KeyModifiers) -> Option<u8> {
    let shifted = modifiers.shift();
    let letter_shift = shifted ^ modifiers.caps_lock;
    Some(match raw & 0x7f {
        0x02 => {
            if shifted {
                b'!'
            } else {
                b'1'
            }
        }
        0x03 => {
            if shifted {
                b'@'
            } else {
                b'2'
            }
        }
        0x04 => {
            if shifted {
                b'#'
            } else {
                b'3'
            }
        }
        0x05 => {
            if shifted {
                b'$'
            } else {
                b'4'
            }
        }
        0x06 => {
            if shifted {
                b'%'
            } else {
                b'5'
            }
        }
        0x07 => {
            if shifted {
                b'^'
            } else {
                b'6'
            }
        }
        0x08 => {
            if shifted {
                b'&'
            } else {
                b'7'
            }
        }
        0x09 => {
            if shifted {
                b'*'
            } else {
                b'8'
            }
        }
        0x0a => {
            if shifted {
                b'('
            } else {
                b'9'
            }
        }
        0x0b => {
            if shifted {
                b')'
            } else {
                b'0'
            }
        }
        0x0c => {
            if shifted {
                b'_'
            } else {
                b'-'
            }
        }
        0x0d => {
            if shifted {
                b'+'
            } else {
                b'='
            }
        }
        0x10 => {
            if letter_shift {
                b'Q'
            } else {
                b'q'
            }
        }
        0x11 => {
            if letter_shift {
                b'W'
            } else {
                b'w'
            }
        }
        0x12 => {
            if letter_shift {
                b'E'
            } else {
                b'e'
            }
        }
        0x13 => {
            if letter_shift {
                b'R'
            } else {
                b'r'
            }
        }
        0x14 => {
            if letter_shift {
                b'T'
            } else {
                b't'
            }
        }
        0x15 => {
            if letter_shift {
                b'Y'
            } else {
                b'y'
            }
        }
        0x16 => {
            if letter_shift {
                b'U'
            } else {
                b'u'
            }
        }
        0x17 => {
            if letter_shift {
                b'I'
            } else {
                b'i'
            }
        }
        0x18 => {
            if letter_shift {
                b'O'
            } else {
                b'o'
            }
        }
        0x19 => {
            if letter_shift {
                b'P'
            } else {
                b'p'
            }
        }
        0x1e => {
            if letter_shift {
                b'A'
            } else {
                b'a'
            }
        }
        0x1f => {
            if letter_shift {
                b'S'
            } else {
                b's'
            }
        }
        0x20 => {
            if letter_shift {
                b'D'
            } else {
                b'd'
            }
        }
        0x21 => {
            if letter_shift {
                b'F'
            } else {
                b'f'
            }
        }
        0x22 => {
            if letter_shift {
                b'G'
            } else {
                b'g'
            }
        }
        0x23 => {
            if letter_shift {
                b'H'
            } else {
                b'h'
            }
        }
        0x24 => {
            if letter_shift {
                b'J'
            } else {
                b'j'
            }
        }
        0x25 => {
            if letter_shift {
                b'K'
            } else {
                b'k'
            }
        }
        0x26 => {
            if letter_shift {
                b'L'
            } else {
                b'l'
            }
        }
        0x27 => {
            if shifted {
                b':'
            } else {
                b';'
            }
        }
        0x28 => {
            if shifted {
                b'"'
            } else {
                b'\''
            }
        }
        0x2b => {
            if shifted {
                b'|'
            } else {
                b'\\'
            }
        }
        0x2c => {
            if letter_shift {
                b'Z'
            } else {
                b'z'
            }
        }
        0x2d => {
            if letter_shift {
                b'X'
            } else {
                b'x'
            }
        }
        0x2e => {
            if letter_shift {
                b'C'
            } else {
                b'c'
            }
        }
        0x2f => {
            if letter_shift {
                b'V'
            } else {
                b'v'
            }
        }
        0x30 => {
            if letter_shift {
                b'B'
            } else {
                b'b'
            }
        }
        0x31 => {
            if letter_shift {
                b'N'
            } else {
                b'n'
            }
        }
        0x32 => {
            if letter_shift {
                b'M'
            } else {
                b'm'
            }
        }
        0x33 => {
            if shifted {
                b'<'
            } else {
                b','
            }
        }
        0x34 => {
            if shifted {
                b'>'
            } else {
                b'.'
            }
        }
        0x35 => {
            if shifted {
                b'?'
            } else {
                b'/'
            }
        }
        0x39 => b' ',
        _ => return None,
    })
}

fn set2_to_set1(byte: u8, extended: bool) -> u16 {
    if extended {
        return match byte {
            0x75 => 0x48,
            0x72 => 0x50,
            0x6b => 0x4b,
            0x74 => 0x4d,
            0x14 => 0x1d,
            0x11 => 0x38,
            0x5a => 0x1c,
            _ => byte as u16,
        };
    }
    match byte {
        0x76 => 0x01,
        0x16 => 0x02,
        0x1e => 0x03,
        0x26 => 0x04,
        0x25 => 0x05,
        0x2e => 0x06,
        0x36 => 0x07,
        0x3d => 0x08,
        0x3e => 0x09,
        0x46 => 0x0a,
        0x45 => 0x0b,
        0x4e => 0x0c,
        0x55 => 0x0d,
        0x66 => 0x0e,
        0x0d => 0x0f,
        0x15 => 0x10,
        0x1d => 0x11,
        0x24 => 0x12,
        0x2d => 0x13,
        0x2c => 0x14,
        0x35 => 0x15,
        0x3c => 0x16,
        0x43 => 0x17,
        0x44 => 0x18,
        0x4d => 0x19,
        0x1c => 0x1e,
        0x1b => 0x1f,
        0x23 => 0x20,
        0x2b => 0x21,
        0x34 => 0x22,
        0x33 => 0x23,
        0x3b => 0x24,
        0x42 => 0x25,
        0x4b => 0x26,
        0x4c => 0x27,
        0x52 => 0x28,
        0x5d => 0x2b,
        0x1a => 0x2c,
        0x22 => 0x2d,
        0x21 => 0x2e,
        0x2a => 0x2f,
        0x32 => 0x30,
        0x31 => 0x31,
        0x3a => 0x32,
        0x41 => 0x33,
        0x49 => 0x34,
        0x4a => 0x35,
        0x12 => 0x2a,
        0x59 => 0x36,
        0x14 => {
            if extended {
                0x1d
            } else {
                0x1d
            }
        }
        0x11 => 0x38,
        0x1f if extended => 0x015b,
        0x58 => 0x3a,
        0x05 => 0x3b,
        0x06 => 0x3c,
        0x04 => 0x3d,
        0x0c => 0x3e,
        0x03 => 0x3f,
        0x0b => 0x40,
        0x83 => 0x41,
        0x0a => 0x42,
        0x01 => 0x43,
        0x09 => 0x44,
        0x78 => 0x57,
        0x07 => 0x58,
        0x5a => 0x1c,
        0x29 => 0x39,
        _ => byte as u16,
    }
}

// Override Char(0) ASCII once raw set-1 identity is known.
impl Ps2Decoder {
    fn update_ascii_with_raw(event: KeyboardEvent) -> KeyboardEvent {
        if event.state == KeyState::Pressed && event.keycode == KeyCode::Char(0) {
            KeyboardEvent {
                ascii: ascii_for_set1_raw(event.raw_code, event.modifiers),
                ..event
            }
        } else {
            event
        }
    }
}

pub static PS2_KEYBOARD_DRIVER: Ps2KeyboardDriver = Ps2KeyboardDriver::new();

#[cfg(test)]
mod tests {
    use super::*;

    fn one(decoder: &mut Ps2Decoder, byte: u8) -> Option<KeyboardEvent> {
        decoder.feed(byte)[0].map(Ps2Decoder::update_ascii_with_raw)
    }

    #[test]
    fn translated_set1_decodes_escape_press_release() {
        let mut decoder = Ps2Decoder::new(Ps2ScanSet::Set1Translated);
        let press = one(&mut decoder, 0x01).unwrap();
        assert_eq!(press.keycode, KeyCode::Escape);
        assert_eq!(press.state, KeyState::Pressed);
        let release = one(&mut decoder, 0x81).unwrap();
        assert_eq!(release.keycode, KeyCode::Escape);
        assert_eq!(release.state, KeyState::Released);
    }

    #[test]
    fn set2_decodes_extended_arrow() {
        let mut decoder = Ps2Decoder::new(Ps2ScanSet::Set2);
        assert!(decoder.feed(0xe0)[0].is_none());
        let event = decoder.feed(0x75)[0].unwrap();
        assert_eq!(event.keycode, KeyCode::ArrowUp);
        assert_eq!(event.state, KeyState::Pressed);
    }

    #[test]
    fn ascii_translation_handles_shift() {
        let mut mods = KeyModifiers::empty();
        assert_eq!(ascii_for_set1_raw(0x1e, mods), Some(b'a'));
        mods.left_shift = true;
        assert_eq!(ascii_for_set1_raw(0x1e, mods), Some(b'A'));
    }

    #[test]
    fn set2_decodes_a_press_release() {
        let mut decoder = Ps2Decoder::new(Ps2ScanSet::Set2);
        let press = decoder.feed(0x1c)[0].unwrap();
        assert_eq!(press.ascii, Some(b'a'));
        assert_eq!(press.state, KeyState::Pressed);
        assert!(decoder.feed(0xf0)[0].is_none());
        let release = decoder.feed(0x1c)[0].unwrap();
        assert_eq!(release.state, KeyState::Released);
    }

    #[test]
    fn set2_decodes_shift_a() {
        let mut decoder = Ps2Decoder::new(Ps2ScanSet::Set2);
        assert_eq!(decoder.feed(0x12)[0].unwrap().keycode, KeyCode::LeftShift);
        let press = decoder.feed(0x1c)[0].unwrap();
        assert_eq!(press.ascii, Some(b'A'));
    }

    #[test]
    fn set2_decodes_escape_enter_backspace() {
        let mut decoder = Ps2Decoder::new(Ps2ScanSet::Set2);
        assert_eq!(decoder.feed(0x76)[0].unwrap().keycode, KeyCode::Escape);
        assert_eq!(decoder.feed(0x5a)[0].unwrap().keycode, KeyCode::Enter);
        assert_eq!(decoder.feed(0x66)[0].unwrap().keycode, KeyCode::Backspace);
    }

    #[test]
    fn set2_caps_lock_toggles_ascii() {
        let mut decoder = Ps2Decoder::new(Ps2ScanSet::Set2);
        assert_eq!(decoder.feed(0x58)[0].unwrap().keycode, KeyCode::CapsLock);
        let press = decoder.feed(0x1c)[0].unwrap();
        assert_eq!(press.ascii, Some(b'A'));
    }

    #[test]
    fn pause_sequence_does_not_poison_decoder() {
        let mut decoder = Ps2Decoder::new(Ps2ScanSet::Set2);
        for byte in [0xe1, 0x14, 0x77, 0xe1, 0xf0, 0x14, 0xf0, 0x77] {
            let _ = decoder.feed(byte);
        }
        let event = decoder.feed(0x76)[0].unwrap();
        assert_eq!(event.keycode, KeyCode::Escape);
    }
}
