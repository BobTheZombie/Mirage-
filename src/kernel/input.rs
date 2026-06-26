//! Common no-heap keyboard/event input layer for early built-in hardware drivers.
//!
//! The layer is deliberately mechanism-only: architecture drivers decode raw
//! hardware reports into [`KeyboardEvent`] values and this module stores them in
//! a fixed bounded queue that can be consumed by the debug-shell path, the boot
//! phase manager diagnostics, or normal device reads.

use core::cmp::min;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::device::{MirageInputEvent, MIRAGE_DEVICE_KIND_INPUT_CONTROLLER};
use crate::kernel::sync::SpinLock;

pub const INPUT_EVENT_TYPE_KEYBOARD: u16 = 1;
pub const INPUT_EVENT_TYPE_VENDOR: u16 = 2;
pub const INPUT_QUEUE_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Escape,
    Enter,
    Backspace,
    Tab,
    Char(u8),
    F(u8),
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    LeftShift,
    RightShift,
    LeftCtrl,
    RightCtrl,
    LeftAlt,
    RightAlt,
    CapsLock,
    VolumeUp,
    VolumeDown,
    Mute,
    BrightnessUp,
    BrightnessDown,
    Sleep,
    Meta,
    NumLock,
    ScrollLock,
    DisplaySwitch,
    Raw(u16),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModifiers {
    pub left_shift: bool,
    pub right_shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
    pub scroll_lock: bool,
}

impl KeyModifiers {
    pub const fn empty() -> Self {
        Self {
            left_shift: false,
            right_shift: false,
            ctrl: false,
            alt: false,
            meta: false,
            caps_lock: false,
            num_lock: false,
            scroll_lock: false,
        }
    }

    pub const fn shift(self) -> bool {
        self.left_shift || self.right_shift
    }

    pub const fn bits(self) -> u16 {
        (self.left_shift as u16)
            | ((self.right_shift as u16) << 1)
            | ((self.ctrl as u16) << 2)
            | ((self.alt as u16) << 3)
            | ((self.meta as u16) << 4)
            | ((self.caps_lock as u16) << 5)
            | ((self.num_lock as u16) << 6)
            | ((self.scroll_lock as u16) << 7)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputRawSource {
    Ps2,
    UsbHid,
    AcpiEc,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyboardEvent {
    pub keycode: KeyCode,
    pub state: KeyState,
    pub modifiers: KeyModifiers,
    pub ascii: Option<u8>,
    pub raw_source: InputRawSource,
    pub raw_code: u16,
}

impl KeyboardEvent {
    pub const fn new(
        keycode: KeyCode,
        state: KeyState,
        modifiers: KeyModifiers,
        ascii: Option<u8>,
        raw_source: InputRawSource,
        raw_code: u16,
    ) -> Self {
        Self {
            keycode,
            state,
            modifiers,
            ascii,
            raw_source,
            raw_code,
        }
    }

    pub fn to_mirage_input_event(self) -> MirageInputEvent {
        MirageInputEvent {
            event_type: match self.keycode {
                KeyCode::Raw(_) if self.raw_source == InputRawSource::AcpiEc => {
                    INPUT_EVENT_TYPE_VENDOR
                }
                _ => INPUT_EVENT_TYPE_KEYBOARD,
            },
            code: encode_keycode(self.keycode, self.ascii),
            value: match self.state {
                KeyState::Pressed => 1,
                KeyState::Released => 0,
            },
            timestamp_ns: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct InputQueue {
    events: [KeyboardEvent; INPUT_QUEUE_CAPACITY],
    head: usize,
    len: usize,
    dropped: u64,
}

impl InputQueue {
    const EMPTY_EVENT: KeyboardEvent = KeyboardEvent::new(
        KeyCode::Raw(0),
        KeyState::Released,
        KeyModifiers::empty(),
        None,
        InputRawSource::Unknown,
        0,
    );

    const fn new() -> Self {
        Self {
            events: [Self::EMPTY_EVENT; INPUT_QUEUE_CAPACITY],
            head: 0,
            len: 0,
            dropped: 0,
        }
    }

    fn push(&mut self, event: KeyboardEvent) {
        if self.len == self.events.len() {
            self.head = (self.head + 1) % self.events.len();
            self.len -= 1;
            self.dropped = self.dropped.saturating_add(1);
        }
        let tail = (self.head + self.len) % self.events.len();
        self.events[tail] = event;
        self.len += 1;
    }

    fn pop(&mut self) -> Option<KeyboardEvent> {
        if self.len == 0 {
            return None;
        }
        let event = self.events[self.head];
        self.head = (self.head + 1) % self.events.len();
        self.len -= 1;
        Some(event)
    }
}

#[derive(Clone, Copy)]
struct InputRegistry {
    ps2_online: bool,
    usb_online: bool,
    ec_online: bool,
}

impl InputRegistry {
    const fn new() -> Self {
        Self {
            ps2_online: false,
            usb_online: false,
            ec_online: false,
        }
    }

    const fn any_online(self) -> bool {
        self.ps2_online || self.usb_online || self.ec_online
    }
}

static INPUT_QUEUE: SpinLock<InputQueue> = SpinLock::new(InputQueue::new());
static INPUT_QUEUE_BUSY_DROPS: AtomicU64 = AtomicU64::new(0);
static INPUT_REGISTRY: SpinLock<InputRegistry> = SpinLock::new(InputRegistry::new());

pub fn publish_keyboard_event(event: KeyboardEvent) {
    INPUT_QUEUE.lock().push(event);
}

/// IRQ-safe producer path.  It never spins on the queue lock; if a normal
/// consumer temporarily owns the queue, the interrupt drops the event and
/// accounts the loss instead of risking an interrupt-time deadlock.
pub fn try_publish_keyboard_event(event: KeyboardEvent) -> bool {
    if let Some(mut queue) = INPUT_QUEUE.try_lock() {
        queue.push(event);
        true
    } else {
        INPUT_QUEUE_BUSY_DROPS.fetch_add(1, Ordering::Relaxed);
        false
    }
}

pub fn input_queue_overflows() -> u64 {
    INPUT_QUEUE
        .lock()
        .dropped
        .saturating_add(INPUT_QUEUE_BUSY_DROPS.load(Ordering::Relaxed))
}

pub fn input_queue_depth() -> usize {
    INPUT_QUEUE.lock().len
}

pub fn pop_keyboard_event() -> Option<KeyboardEvent> {
    INPUT_QUEUE.lock().pop()
}

pub fn copy_mirage_events(buffer: &mut [u8]) -> usize {
    let event_size = core::mem::size_of::<MirageInputEvent>();
    if buffer.len() < event_size {
        return 0;
    }

    let event_capacity = buffer.len() / event_size;
    let mut written = 0usize;
    let mut queue = INPUT_QUEUE.lock();
    while written < min(event_capacity, INPUT_QUEUE_CAPACITY) {
        let Some(event) = queue.pop() else { break };
        let abi = event.to_mirage_input_event();
        let bytes = unsafe {
            core::slice::from_raw_parts(
                core::ptr::addr_of!(abi) as *const u8,
                core::mem::size_of::<MirageInputEvent>(),
            )
        };
        buffer[written * event_size..][..event_size].copy_from_slice(bytes);
        written += 1;
    }
    written * event_size
}

pub fn mark_source_online(source: InputRawSource) {
    let mut registry = INPUT_REGISTRY.lock();
    match source {
        InputRawSource::Ps2 => registry.ps2_online = true,
        InputRawSource::UsbHid => registry.usb_online = true,
        InputRawSource::AcpiEc => registry.ec_online = true,
        InputRawSource::Unknown => {}
    }
}

pub fn any_keyboard_online() -> bool {
    INPUT_REGISTRY.lock().any_online()
}

pub fn poll_debug_escape() -> bool {
    let mut found = false;
    let mut requeue = [InputQueue::EMPTY_EVENT; 8];
    let mut requeue_len = 0usize;

    while let Some(event) = pop_keyboard_event() {
        if event.keycode == KeyCode::Escape && event.state == KeyState::Pressed {
            found = true;
            break;
        }
        if requeue_len < requeue.len() {
            requeue[requeue_len] = event;
            requeue_len += 1;
        }
    }

    let mut index = 0usize;
    while index < requeue_len {
        publish_keyboard_event(requeue[index]);
        index += 1;
    }

    found
}

pub const fn encode_keycode(keycode: KeyCode, ascii: Option<u8>) -> u16 {
    match keycode {
        KeyCode::Escape => 0x0001,
        KeyCode::Enter => 0x001c,
        KeyCode::Backspace => 0x000e,
        KeyCode::Tab => 0x000f,
        KeyCode::Char(_) => match ascii {
            Some(byte) => 0x0200 | byte as u16,
            None => 0x0200,
        },
        KeyCode::F(n) => 0x0300 | n as u16,
        KeyCode::Insert => 0x5200,
        KeyCode::Delete => 0x5300,
        KeyCode::Home => 0x4700,
        KeyCode::End => 0x4f00,
        KeyCode::PageUp => 0x4900,
        KeyCode::PageDown => 0x5100,
        KeyCode::ArrowUp => 0x4800,
        KeyCode::ArrowDown => 0x5000,
        KeyCode::ArrowLeft => 0x4b00,
        KeyCode::ArrowRight => 0x4d00,
        KeyCode::LeftShift => 0x002a,
        KeyCode::RightShift => 0x0036,
        KeyCode::LeftCtrl => 0x001d,
        KeyCode::RightCtrl => 0x011d,
        KeyCode::LeftAlt => 0x0038,
        KeyCode::RightAlt => 0x0138,
        KeyCode::CapsLock => 0x003a,
        KeyCode::VolumeUp => 0x0401,
        KeyCode::VolumeDown => 0x0402,
        KeyCode::Mute => 0x0403,
        KeyCode::BrightnessUp => 0x0404,
        KeyCode::BrightnessDown => 0x0405,
        KeyCode::Sleep => 0x0406,
        KeyCode::Meta => 0x0408,
        KeyCode::NumLock => 0x0045,
        KeyCode::ScrollLock => 0x0046,
        KeyCode::DisplaySwitch => 0x0407,
        KeyCode::Raw(code) => code,
    }
}

pub const fn device_kind_tag() -> u32 {
    MIRAGE_DEVICE_KIND_INPUT_CONTROLLER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_event_converts_to_mirage_input_event() {
        let event = KeyboardEvent::new(
            KeyCode::Escape,
            KeyState::Pressed,
            KeyModifiers::empty(),
            None,
            InputRawSource::Ps2,
            1,
        );
        let abi = event.to_mirage_input_event();
        assert_eq!(abi.event_type, INPUT_EVENT_TYPE_KEYBOARD);
        assert_eq!(abi.code, 1);
        assert_eq!(abi.value, 1);
    }

    #[test]
    fn queue_overflow_drops_oldest() {
        let mut queue = InputQueue::new();
        for code in 0..(INPUT_QUEUE_CAPACITY + 1) {
            queue.push(KeyboardEvent::new(
                KeyCode::Raw(code as u16),
                KeyState::Pressed,
                KeyModifiers::empty(),
                None,
                InputRawSource::Unknown,
                code as u16,
            ));
        }
        assert_eq!(queue.dropped, 1);
        assert_eq!(queue.pop().unwrap().raw_code, 1);
    }
}
