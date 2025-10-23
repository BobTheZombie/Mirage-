//! Core kernel primitives: process lifecycle, scheduling, IPC routing, and
//! multi-core orchestration.

pub mod cpu;
pub mod ipc;
pub mod memory;
pub mod process;
pub mod scheduler;
pub mod sync;
pub mod thread;
pub mod time;

use crate::arch::x86_64::clock;
use crate::kernel::cpu::CpuCoreState;
use crate::kernel::ipc::{Message, MessagePayload, MessageQueue, MessageQueueError};
use crate::kernel::process::{ProcessControlBlock, ProcessId, ProcessPriority, ProcessState};
use crate::kernel::scheduler::{ScheduledThread, Scheduler};
use crate::kernel::thread::{ThreadControlBlock, ThreadId, ThreadState, MAX_THREADS};
use crate::kernel::time::KERNEL_TIME;
use crate::subkernel::{Credentials, SecurityKernel};

pub const MAX_PROCESSES: usize = 64;
pub const MESSAGE_DEPTH: usize = 16;

#[derive(Debug, Clone, Copy)]
pub enum KernelError {
    ProcessTableFull,
    SchedulerFull,
    UnknownProcess,
    UnknownThread,
    ThreadTableFull,
    MessageQueueFull,
    MessageQueueEmpty,
    SecurityViolation,
    IsolationFault,
}

pub type KernelResult<T> = core::result::Result<T, KernelError>;

pub struct Kernel<const MAX_PROC: usize, const MSG_DEPTH: usize> {
    process_table: [Option<ProcessControlBlock>; MAX_PROC],
    ipc_queues: [MessageQueue<MSG_DEPTH>; MAX_PROC],
    scheduler: Scheduler<MAX_THREADS>,
    security: SecurityKernel<MAX_PROC>,
    core_states: [CpuCoreState; cpu::MAX_CORES],
    thread_table: [Option<ThreadControlBlock>; MAX_THREADS],
    next_pid: u64,
    next_thread: u64,
    message_sequence: u64,
}

impl<const MAX_PROC: usize, const MSG_DEPTH: usize> Kernel<MAX_PROC, MSG_DEPTH> {
    const THREAD_CAPACITY: usize = MAX_THREADS;

    pub const fn new() -> Self {
        Self {
            process_table: [None; MAX_PROC],
            ipc_queues: [MessageQueue::new(); MAX_PROC],
            scheduler: Scheduler::new(),
            security: SecurityKernel::new(),
            core_states: [CpuCoreState::new(); cpu::MAX_CORES],
            thread_table: [None; MAX_THREADS],
            next_pid: 1,
            next_thread: 1,
            message_sequence: 0,
        }
    }

    pub fn bootstrap(&mut self) {
        self.scheduler.reset();
        self.security.reset();
        self.next_pid = 1;
        self.next_thread = 1;
        self.message_sequence = 0;
        KERNEL_TIME.init(clock::DEFAULT_FREQUENCY_HZ);

        let mut idx = 0;
        while idx < MAX_PROC {
            self.process_table[idx] = None;
            self.ipc_queues[idx].clear();
            idx += 1;
        }

        idx = 0;
        while idx < Self::THREAD_CAPACITY {
            self.thread_table[idx] = None;
            idx += 1;
        }

        idx = 0;
        while idx < cpu::MAX_CORES {
            self.core_states[idx] = CpuCoreState::new();
            idx += 1;
        }
        if cpu::MAX_CORES > 0 {
            self.core_states[0].online();
        }
    }

    pub fn bring_up_secondary_cores(&mut self, count: usize) {
        let mut brought_online = 0usize;
        let mut idx = 1usize;
        while idx < cpu::MAX_CORES && brought_online < count {
            self.core_states[idx].online();
            brought_online += 1;
            idx += 1;
        }
    }

    pub fn online_core_count(&self) -> usize {
        let mut count = 0usize;
        let mut idx = 0usize;
        while idx < cpu::MAX_CORES {
            if self.core_states[idx].online {
                count += 1;
            }
            idx += 1;
        }
        count
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

        self.process_table[slot] = Some(pcb);

        let thread_id = match self.create_thread(pid, entry_point, priority) {
            Ok(id) => id,
            Err(err) => {
                self.process_table[slot] = None;
                self.security.revoke_task(pid);
                return Err(err);
            }
        };

        if let Some(pcb) = self.process_table[slot].as_mut() {
            pcb.state = ProcessState::Ready;
        }

        if self
            .scheduler
            .enqueue(ScheduledThread::new(thread_id, pid, priority))
            .is_err()
        {
            self.rollback_thread_creation(thread_id);
            self.process_table[slot] = None;
            self.security.revoke_task(pid);
            return Err(KernelError::SchedulerFull);
        }

        Ok(pid)
    }

    pub fn spawn_thread(
        &mut self,
        pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
    ) -> KernelResult<ThreadId> {
        self.ensure_process_exists(pid)?;
        let thread_id = self.create_thread(pid, entry_point, priority)?;
        if self
            .scheduler
            .enqueue(ScheduledThread::new(thread_id, pid, priority))
            .is_err()
        {
            self.rollback_thread_creation(thread_id);
            return Err(KernelError::SchedulerFull);
        }
        Ok(thread_id)
    }

    pub fn terminate_process(&mut self, pid: ProcessId) {
        if let Ok(index) = self.locate_process(pid) {
            self.process_table[index] = None;
            self.ipc_queues[index].clear();
            self.scheduler.remove_process(pid);
            self.remove_threads_for_process(pid);
            self.security.revoke_task(pid);
        }
    }

    pub fn terminate_thread(&mut self, thread: ThreadId) {
        if let Ok(index) = self.locate_thread(thread) {
            if let Some(tcb) = self.thread_table[index] {
                self.scheduler.remove_thread(thread);
                self.remove_thread_from_cores(thread);
                self.thread_table[index] = None;
                self.update_process_thread_count(tcb.process, false);
            }
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

        let mut wake_threads = false;
        if let Some(pcb) = self.process_table[queue_index].as_mut() {
            if pcb.state == ProcessState::Blocked {
                pcb.state = ProcessState::Ready;
                wake_threads = true;
            }
        }

        if wake_threads {
            self.make_threads_ready(receiver);
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
            }
            self.scheduler.remove_process(pid);
            self.block_threads_for_process(pid);
        }
    }

    pub fn tick(&mut self) {
        let _timestamp = KERNEL_TIME.tick();
        let mut core_index = 0usize;
        while core_index < cpu::MAX_CORES {
            if self.core_states[core_index].online {
                self.run_core(core_index);
            }
            core_index += 1;
        }
    }

    fn run_core(&mut self, core_index: usize) {
        if let Some(mut scheduled) = self.scheduler.next() {
            let thread_index = match self.locate_thread(scheduled.thread) {
                Ok(idx) => idx,
                Err(_) => {
                    self.core_states[core_index].idle_cycle();
                    return;
                }
            };

            let process_index = match self.locate_process(scheduled.process) {
                Ok(idx) => idx,
                Err(_) => {
                    self.thread_table[thread_index] = None;
                    self.core_states[core_index].idle_cycle();
                    return;
                }
            };

            if self.security.enforce_isolation(scheduled.process).is_err() {
                self.handle_isolation_fault(scheduled.process);
                return;
            }

            self.core_states[core_index].start_thread(scheduled.thread);

            let mut terminated = false;
            if let Some(entry) = self.thread_table.get_mut(thread_index) {
                if let Some(thread) = entry.as_mut() {
                    if thread.state == ThreadState::Terminated {
                        *entry = None;
                        terminated = true;
                    } else {
                        thread.mark_running();
                        thread.accumulate_cpu_time(1);
                    }
                }
            }

            if terminated {
                self.update_process_thread_count(scheduled.process, false);
                self.core_states[core_index].finish_cycle();
                return;
            }

            if let Some(pcb) = self.process_table[process_index].as_mut() {
                pcb.state = ProcessState::Running;
                pcb.cpu_time = pcb.cpu_time.saturating_add(1);
            }

            if let Some(entry) = self.thread_table.get_mut(thread_index) {
                if let Some(thread) = entry.as_mut() {
                    thread.mark_ready();
                }
            }

            if let Some(pcb) = self.process_table[process_index].as_mut() {
                pcb.state = ProcessState::Ready;
            }

            self.core_states[core_index].finish_cycle();

            if scheduled.consume_time_slice() {
                scheduled.reset_time_slice();
            }

            let _ = self.scheduler.requeue(scheduled);
        } else {
            self.core_states[core_index].idle_cycle();
        }
    }

    fn block_threads_for_process(&mut self, pid: ProcessId) {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(entry) = self.thread_table.get_mut(idx) {
                if let Some(thread) = entry.as_mut() {
                    if thread.process == pid {
                        thread.block();
                    }
                }
            }
            idx += 1;
        }
    }

    fn make_threads_ready(&mut self, pid: ProcessId) {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(entry) = self.thread_table.get_mut(idx) {
                if let Some(thread) = entry.as_mut() {
                    if thread.process == pid && thread.state == ThreadState::Blocked {
                        thread.mark_ready();
                        let _ = self.scheduler.enqueue(ScheduledThread::new(
                            thread.id,
                            thread.process,
                            thread.priority,
                        ));
                    }
                }
            }
            idx += 1;
        }
    }

    fn remove_threads_for_process(&mut self, pid: ProcessId) {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(thread) = self.thread_table[idx] {
                if thread.process == pid {
                    self.scheduler.remove_thread(thread.id);
                    self.remove_thread_from_cores(thread.id);
                    self.thread_table[idx] = None;
                }
            }
            idx += 1;
        }
    }

    fn remove_thread_from_cores(&mut self, thread: ThreadId) {
        let mut idx = 0usize;
        while idx < cpu::MAX_CORES {
            self.core_states[idx].evict(thread);
            idx += 1;
        }
    }

    fn create_thread(
        &mut self,
        pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
    ) -> KernelResult<ThreadId> {
        let slot = self
            .find_free_thread_slot()
            .ok_or(KernelError::ThreadTableFull)?;
        let id = self.allocate_thread_id();
        let tcb = ThreadControlBlock::new(id, pid, entry_point, priority);
        self.thread_table[slot] = Some(tcb);
        self.update_process_thread_count(pid, true);
        Ok(id)
    }

    fn rollback_thread_creation(&mut self, thread: ThreadId) {
        if let Ok(index) = self.locate_thread(thread) {
            if let Some(tcb) = self.thread_table[index] {
                self.thread_table[index] = None;
                self.update_process_thread_count(tcb.process, false);
            }
        }
    }

    fn update_process_thread_count(&mut self, pid: ProcessId, increment: bool) {
        if let Ok(index) = self.locate_process(pid) {
            if let Some(pcb) = self.process_table[index].as_mut() {
                if increment {
                    pcb.increment_thread_count();
                } else {
                    pcb.decrement_thread_count();
                }
            }
        }
    }

    fn ensure_process_exists(&self, pid: ProcessId) -> KernelResult<()> {
        self.locate_process(pid).map(|_| ())
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

    fn find_free_thread_slot(&self) -> Option<usize> {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if self.thread_table[idx].is_none() {
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

    fn locate_thread(&self, thread: ThreadId) -> KernelResult<usize> {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(tcb) = &self.thread_table[idx] {
                if tcb.id == thread {
                    return Ok(idx);
                }
            }
            idx += 1;
        }
        Err(KernelError::UnknownThread)
    }

    fn allocate_pid(&mut self) -> ProcessId {
        let pid = ProcessId::new(self.next_pid);
        self.next_pid += 1;
        pid
    }

    fn allocate_thread_id(&mut self) -> ThreadId {
        let id = ThreadId::new(self.next_thread);
        self.next_thread += 1;
        id
    }

    fn next_message_sequence(&mut self) -> u64 {
        let seq = self.message_sequence;
        self.message_sequence = self.message_sequence.wrapping_add(1);
        seq
    }
}
