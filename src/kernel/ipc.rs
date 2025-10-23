//! Inter-process communication primitives.

use crate::kernel::process::ProcessId;
use crate::subkernel::SecurityClass;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessagePayload {
    pub security_class: SecurityClass,
    pub data: [u8; 64],
    pub length: usize,
}

impl MessagePayload {
    pub const fn empty(security_class: SecurityClass) -> Self {
        Self {
            security_class,
            data: [0; 64],
            length: 0,
        }
    }

    pub fn from_slice(security_class: SecurityClass, slice: &[u8]) -> Self {
        let mut payload = Self::empty(security_class);
        let mut idx = 0;
        while idx < slice.len() && idx < payload.data.len() {
            payload.data[idx] = slice[idx];
            idx += 1;
        }
        payload.length = idx;
        payload
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub sender: ProcessId,
    pub receiver: ProcessId,
    pub sequence: u64,
    pub payload: MessagePayload,
}

impl Message {
    pub const fn new(
        sender: ProcessId,
        receiver: ProcessId,
        sequence: u64,
        payload: MessagePayload,
    ) -> Self {
        Self {
            sender,
            receiver,
            sequence,
            payload,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageQueueError {
    Full,
}

#[derive(Clone, Copy)]
pub struct MessageQueue<const N: usize> {
    buffer: [Option<Message>; N],
    head: usize,
    tail: usize,
    len: usize,
}

impl<const N: usize> MessageQueue<N> {
    pub const fn new() -> Self {
        Self {
            buffer: [None; N],
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, message: Message) -> Result<(), MessageQueueError> {
        if self.is_full() {
            return Err(MessageQueueError::Full);
        }
        self.buffer[self.tail] = Some(message);
        self.tail = (self.tail + 1) % N;
        self.len += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Option<Message> {
        if self.len == 0 {
            return None;
        }
        let message = self.buffer[self.head];
        self.buffer[self.head] = None;
        self.head = (self.head + 1) % N;
        self.len -= 1;
        message
    }

    pub fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.len = 0;
        let mut idx = 0;
        while idx < N {
            self.buffer[idx] = None;
            idx += 1;
        }
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_full(&self) -> bool {
        self.len == N
    }
}
