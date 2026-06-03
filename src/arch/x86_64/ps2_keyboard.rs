//! Polling PS/2 keyboard input driver backed by a bounded MirageInputEvent queue.

use core::cmp::min;

use crate::arch::x86_64::io::inb;
use crate::kernel::device::{
    copy_input_event_to_bytes, DeviceDriver, DeviceError, DeviceKind, MirageInputEvent,
};
use crate::kernel::sync::SpinLock;
use crate::subkernel::{DeviceSecurity, SecurityClass};

const DATA_PORT: u16 = 0x60;
const STATUS_PORT: u16 = 0x64;
const STATUS_OUTPUT_FULL: u8 = 0x01;
const STATUS_AUX_DATA: u8 = 0x20;
const MAX_POLL_BYTES: usize = 32;
const EVENT_TYPE_KEY: u16 = 1;

pub struct Ps2KeyboardDriver {
    queue: SpinLock<InputQueue>,
}

#[derive(Clone, Copy)]
struct InputQueue {
    events: [MirageInputEvent; Ps2KeyboardDriver::QUEUE_CAPACITY],
    head: usize,
    len: usize,
    extended: bool,
}

impl InputQueue {
    const fn new() -> Self {
        Self {
            events: [MirageInputEvent {
                event_type: 0,
                code: 0,
                value: 0,
                timestamp_ns: 0,
            }; Ps2KeyboardDriver::QUEUE_CAPACITY],
            head: 0,
            len: 0,
            extended: false,
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

impl Ps2KeyboardDriver {
    pub const QUEUE_CAPACITY: usize = 64;

    pub const fn new() -> Self {
        Self {
            queue: SpinLock::new(InputQueue::new()),
        }
    }

    fn poll_hardware(&self) {
        let mut queue = self.queue.lock();
        let mut polls = 0usize;
        while polls < MAX_POLL_BYTES {
            let status = unsafe { inb(STATUS_PORT) };
            if status & STATUS_OUTPUT_FULL == 0 {
                break;
            }
            let scancode = unsafe { inb(DATA_PORT) };
            if status & STATUS_AUX_DATA == 0 {
                Self::decode_scancode(&mut queue, scancode);
            }
            polls += 1;
        }
    }

    fn decode_scancode(queue: &mut InputQueue, scancode: u8) {
        if scancode == 0xe0 {
            queue.extended = true;
            return;
        }
        if scancode == 0xe1 {
            queue.extended = false;
            return;
        }

        let released = scancode & 0x80 != 0;
        let base_code = (scancode & 0x7f) as u16;
        let code = if queue.extended {
            0x0100 | base_code
        } else {
            base_code
        };
        queue.extended = false;
        queue.push(MirageInputEvent {
            event_type: EVENT_TYPE_KEY,
            code,
            value: if released { 0 } else { 1 },
            timestamp_ns: 0,
        });
    }
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
        let mut queue = self.queue.lock();
        while written < min(event_capacity, Self::QUEUE_CAPACITY) {
            let Some(event) = queue.pop() else { break };
            copy_input_event_to_bytes(&event, &mut buffer[written * event_size..][..event_size])?;
            written += 1;
        }
        Ok(written * event_size)
    }
}

pub static PS2_KEYBOARD_DRIVER: Ps2KeyboardDriver = Ps2KeyboardDriver::new();
