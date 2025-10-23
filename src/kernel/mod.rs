//! Core kernel primitives: process lifecycle, scheduling, and IPC routing.

pub mod ipc;
pub mod process;
pub mod scheduler;

use crate::kernel::ipc::{Message, MessagePayload, MessageQueue, MessageQueueError};
use crate::kernel::process::{ProcessControlBlock, ProcessId, ProcessPriority, ProcessState};
use crate::kernel::scheduler::{ScheduledProcess, Scheduler};
use crate::subkernel::{Credentials, SecurityKernel};

pub const MAX_PROCESSES: usize = 64;
pub const MESSAGE_DEPTH: usize = 16;

#[derive(Debug, Clone, Copy)]
pub enum KernelError {
    ProcessTableFull,
    SchedulerFull,
    UnknownProcess,
    MessageQueueFull,
    MessageQueueEmpty,
    SecurityViolation,
    IsolationFault,
}

pub type KernelResult<T> = core::result::Result<T, KernelError>;

pub struct Kernel<const MAX_PROC: usize, const MSG_DEPTH: usize> {
    process_table: [Option<ProcessControlBlock>; MAX_PROC],
    ipc_queues: [MessageQueue<MSG_DEPTH>; MAX_PROC],
    scheduler: Scheduler<MAX_PROC>,
    security: SecurityKernel<MAX_PROC>,
    next_pid: u64,
    message_sequence: u64,
}

impl<const MAX_PROC: usize, const MSG_DEPTH: usize> Kernel<MAX_PROC, MSG_DEPTH> {
    pub const fn new() -> Self {
        Self {
            process_table: [None; MAX_PROC],
            ipc_queues: [MessageQueue::new(); MAX_PROC],
            scheduler: Scheduler::new(),
            security: SecurityKernel::new(),
            next_pid: 1,
            message_sequence: 0,
        }
    }

    pub fn bootstrap(&mut self) {
        self.scheduler.reset();
        self.security.reset();
        self.next_pid = 1;
        self.message_sequence = 0;
        let mut idx = 0;
        while idx < MAX_PROC {
            self.process_table[idx] = None;
            self.ipc_queues[idx].clear();
            idx += 1;
        }
    }

    pub fn spawn_initial_process(&mut self, creds: Credentials) -> KernelResult<ProcessId> {
        self.spawn_process(0, ProcessPriority::Critical, None, creds)
    }

    pub fn spawn_process(
        &mut self,
        entry_point: u64,
        priority: ProcessPriority,
        parent: Option<ProcessId>,
        creds: Credentials,
    ) -> KernelResult<ProcessId> {
        let slot = self.find_free_slot().ok_or(KernelError::ProcessTableFull)?;
        let pid = self.allocate_pid();
        let mut pcb = ProcessControlBlock::new(pid, entry_point, priority, parent);
        pcb.update_security_label(creds.label());

        self.security
            .register_task(pid, creds)
            .map_err(|_| KernelError::SecurityViolation)?;

        self.scheduler
            .enqueue(ScheduledProcess::new(pid, priority))
            .map_err(|_| KernelError::SchedulerFull)?;

        self.process_table[slot] = Some(pcb);
        Ok(pid)
    }

    pub fn terminate_process(&mut self, pid: ProcessId) {
        if let Ok(index) = self.locate_process(pid) {
            self.process_table[index] = None;
            self.ipc_queues[index].clear();
            self.scheduler.remove(pid);
            self.security.revoke_task(pid);
        }
    }

    pub fn send_message(
        &mut self,
        sender: ProcessId,
        receiver: ProcessId,
        payload: MessagePayload,
    ) -> KernelResult<()> {
        self.security
            .authorize_ipc(sender, receiver, payload.security_class)
            .map_err(|_| KernelError::SecurityViolation)?;

        let message = Message::new(sender, receiver, self.next_message_sequence(), payload);
        let queue_index = self.locate_process(receiver)?;
        self.ipc_queues[queue_index]
            .push(message)
            .map_err(|MessageQueueError::Full| KernelError::MessageQueueFull)?;

        if let Some(pcb) = self.process_table[queue_index].as_mut() {
            if pcb.state == ProcessState::Blocked {
                pcb.state = ProcessState::Ready;
                let _ = self
                    .scheduler
                    .enqueue(ScheduledProcess::new(pcb.pid, pcb.priority));
            }
        }

        Ok(())
    }

    pub fn receive_message(&mut self, pid: ProcessId) -> KernelResult<Message> {
        let queue_index = self.locate_process(pid)?;
        self.ipc_queues[queue_index]
            .pop()
            .ok_or(KernelError::MessageQueueEmpty)
    }

    pub fn block_for_message(&mut self, pid: ProcessId) {
        if let Ok(index) = self.locate_process(pid) {
            if let Some(pcb) = self.process_table[index].as_mut() {
                pcb.state = ProcessState::Blocked;
                self.scheduler.remove(pid);
            }
        }
    }

    pub fn tick(&mut self) {
        if let Some(mut scheduled) = self.scheduler.next() {
            if let Ok(index) = self.locate_process(scheduled.pid) {
                if let Some(pcb) = self.process_table[index].as_mut() {
                    pcb.state = ProcessState::Running;
                    if self.security.enforce_isolation(scheduled.pid).is_err() {
                        self.handle_isolation_fault(scheduled.pid);
                        return;
                    }

                    pcb.cpu_time += 1;
                    if scheduled.consume_time_slice() {
                        scheduled.reset_time_slice();
                    }
                    pcb.state = ProcessState::Ready;

                    let _ = self.scheduler.requeue(scheduled);
                }
            }
        }
    }

    fn handle_isolation_fault(&mut self, pid: ProcessId) {
        self.terminate_process(pid);
    }

    fn find_free_slot(&self) -> Option<usize> {
        let mut idx = 0;
        while idx < MAX_PROC {
            if self.process_table[idx].is_none() {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    fn locate_process(&self, pid: ProcessId) -> KernelResult<usize> {
        let mut idx = 0;
        while idx < MAX_PROC {
            if let Some(pcb) = &self.process_table[idx] {
                if pcb.pid == pid {
                    return Ok(idx);
                }
            }
            idx += 1;
        }
        Err(KernelError::UnknownProcess)
    }

    fn allocate_pid(&mut self) -> ProcessId {
        let pid = ProcessId::new(self.next_pid);
        self.next_pid += 1;
        pid
    }

    fn next_message_sequence(&mut self) -> u64 {
        let seq = self.message_sequence;
        self.message_sequence = self.message_sequence.wrapping_add(1);
        seq
    }
}
