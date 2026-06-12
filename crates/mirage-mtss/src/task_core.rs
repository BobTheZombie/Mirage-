//! Fixed-capacity MTSS task/thread core for the first userspace launch milestone.
//!
//! This module keeps the portable scheduler objects in MTSS while leaving CPU
//! privilege transitions, CR3 installation, and `iretq`/`sysret` mechanics to
//! the architecture backend.  It is deliberately allocation-free and suitable
//! for `no_std` kernel use.

use crate::types::AddressSpaceId;

pub const DEFAULT_TASK_TABLE_SIZE: usize = 64;
pub const DEFAULT_THREAD_TABLE_SIZE: usize = 128;
pub const DEFAULT_READY_QUEUE_SIZE: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CoreTaskId(pub u64);

impl CoreTaskId {
    pub const IDLE: Self = Self(0);
    pub const FIRST_USERSPACE: Self = Self(1);

    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CoreThreadId(pub u64);

impl CoreThreadId {
    pub const IDLE: Self = Self(0);

    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum TaskKind {
    Kernel,
    Userspace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum CoreTaskState {
    New,
    Ready,
    Running,
    Blocked,
    Sleeping,
    Exited,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct StackRange {
    pub bottom: u64,
    pub top: u64,
}

impl StackRange {
    pub const fn new(bottom: u64, top: u64) -> Self {
        Self { bottom, top }
    }

    pub const fn len(self) -> u64 {
        self.top.saturating_sub(self.bottom)
    }

    pub const fn is_valid(self) -> bool {
        self.bottom < self.top
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SavedRegisters {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CpuContext {
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
    pub cr3: u64,
    pub regs: SavedRegisters,
}

impl CpuContext {
    pub const USER_RFLAGS: u64 = 0x202;

    pub const fn kernel_placeholder(rip: u64, rsp: u64) -> Self {
        Self {
            rip,
            rsp,
            rflags: Self::USER_RFLAGS,
            cr3: 0,
            regs: SavedRegisters::zeroed(),
        }
    }

    pub const fn userspace_entry(entry: u64, user_stack_top: u64, cr3: u64) -> Self {
        Self {
            rip: entry,
            rsp: user_stack_top,
            rflags: Self::USER_RFLAGS,
            cr3,
            regs: SavedRegisters::zeroed(),
        }
    }
}

impl SavedRegisters {
    pub const fn zeroed() -> Self {
        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CoreTask {
    pub id: CoreTaskId,
    pub kind: TaskKind,
    pub state: CoreTaskState,
    pub address_space: Option<AddressSpaceId>,
    pub main_thread: CoreThreadId,
    pub name: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CoreThread {
    pub id: CoreThreadId,
    pub task: CoreTaskId,
    pub state: CoreTaskState,
    pub kernel_stack: StackRange,
    pub user_stack: Option<StackRange>,
    pub context: CpuContext,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserProgramImage {
    pub entry: u64,
    pub address_space: AddressSpaceId,
    pub user_stack: StackRange,
    pub cr3: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreMtssError {
    TaskTableFull,
    ThreadTableFull,
    ReadyQueueFull,
    ReadyQueueEmpty,
    InvalidAddressSpace,
    InvalidEntry,
    InvalidStack,
    UnknownTask,
    UnknownThread,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreMtss<
    const TASKS: usize = DEFAULT_TASK_TABLE_SIZE,
    const THREADS: usize = DEFAULT_THREAD_TABLE_SIZE,
    const READY: usize = DEFAULT_READY_QUEUE_SIZE,
> {
    next_pid: u64,
    next_tid: u64,
    tasks: [Option<CoreTask>; TASKS],
    threads: [Option<CoreThread>; THREADS],
    ready: [Option<CoreThreadId>; READY],
    ready_head: usize,
    ready_tail: usize,
    ready_len: usize,
    current: Option<CoreThreadId>,
    initialized: bool,
}

impl<const TASKS: usize, const THREADS: usize, const READY: usize> CoreMtss<TASKS, THREADS, READY> {
    pub const fn new() -> Self {
        Self {
            next_pid: 1,
            next_tid: 1,
            tasks: [None; TASKS],
            threads: [None; THREADS],
            ready: [None; READY],
            ready_head: 0,
            ready_tail: 0,
            ready_len: 0,
            current: None,
            initialized: false,
        }
    }

    pub fn init_with_idle(&mut self, kernel_stack: StackRange) -> Result<(), CoreMtssError> {
        if self.initialized {
            return Ok(());
        }
        if !kernel_stack.is_valid() {
            return Err(CoreMtssError::InvalidStack);
        }
        let idle_task = CoreTask {
            id: CoreTaskId::IDLE,
            kind: TaskKind::Kernel,
            state: CoreTaskState::Ready,
            address_space: None,
            main_thread: CoreThreadId::IDLE,
            name: "idle",
        };
        let idle_thread = CoreThread {
            id: CoreThreadId::IDLE,
            task: CoreTaskId::IDLE,
            state: CoreTaskState::Ready,
            kernel_stack,
            user_stack: None,
            context: CpuContext::kernel_placeholder(0, kernel_stack.top),
        };
        self.insert_task(idle_task)?;
        self.insert_thread(idle_thread)?;
        self.enqueue(CoreThreadId::IDLE)?;
        self.initialized = true;
        Ok(())
    }

    pub fn spawn_userspace(
        &mut self,
        name: &'static str,
        program: UserProgramImage,
        kernel_stack: StackRange,
    ) -> Result<CoreTaskId, CoreMtssError> {
        if program.address_space.raw() == 0 || program.cr3 == 0 {
            return Err(CoreMtssError::InvalidAddressSpace);
        }
        if !is_canonical_user(program.entry) {
            return Err(CoreMtssError::InvalidEntry);
        }
        if !program.user_stack.is_valid() || !is_canonical_user(program.user_stack.top) {
            return Err(CoreMtssError::InvalidStack);
        }
        if !kernel_stack.is_valid() {
            return Err(CoreMtssError::InvalidStack);
        }

        let task_id = self.allocate_pid();
        let thread_id = self.allocate_tid();
        let task = CoreTask {
            id: task_id,
            kind: TaskKind::Userspace,
            state: CoreTaskState::Ready,
            address_space: Some(program.address_space),
            main_thread: thread_id,
            name,
        };
        let thread = CoreThread {
            id: thread_id,
            task: task_id,
            state: CoreTaskState::Ready,
            kernel_stack,
            user_stack: Some(program.user_stack),
            context: CpuContext::userspace_entry(
                program.entry,
                program.user_stack.top,
                program.cr3,
            ),
        };
        self.insert_task(task)?;
        self.insert_thread(thread)?;
        self.enqueue(thread_id)?;
        Ok(task_id)
    }

    pub fn on_timer_tick(&mut self) -> Result<Option<CoreThreadId>, CoreMtssError> {
        self.schedule_next()
    }

    pub fn schedule_next(&mut self) -> Result<Option<CoreThreadId>, CoreMtssError> {
        let next = match self.dequeue() {
            Ok(next) => next,
            Err(CoreMtssError::ReadyQueueEmpty) => return Ok(None),
            Err(error) => return Err(error),
        };
        if let Some(current) = self.current {
            if let Some(thread) = self.thread_mut(current) {
                if matches!(thread.state, CoreTaskState::Running) {
                    thread.state = CoreTaskState::Ready;
                    let _ = self.enqueue(current);
                }
            }
        }
        let mut task_to_run = None;
        if let Some(thread) = self.thread_mut(next) {
            thread.state = CoreTaskState::Running;
            task_to_run = Some(thread.task);
        }
        if let Some(task_id) = task_to_run {
            if let Some(task) = self.task_mut(task_id) {
                task.state = CoreTaskState::Running;
            }
        }
        self.current = Some(next);
        Ok(Some(next))
    }

    pub fn exit_current(&mut self, _status: i32) -> Result<Option<CoreThreadId>, CoreMtssError> {
        let Some(current) = self.current.take() else {
            return Ok(None);
        };
        let task = if let Some(thread) = self.thread_mut(current) {
            thread.state = CoreTaskState::Exited;
            thread.task
        } else {
            return Err(CoreMtssError::UnknownThread);
        };
        if let Some(task) = self.task_mut(task) {
            task.state = CoreTaskState::Exited;
        }
        Ok(Some(current))
    }

    pub fn task(&self, id: CoreTaskId) -> Option<CoreTask> {
        self.tasks
            .iter()
            .flatten()
            .copied()
            .find(|task| task.id == id)
    }

    pub fn thread(&self, id: CoreThreadId) -> Option<CoreThread> {
        self.threads
            .iter()
            .flatten()
            .copied()
            .find(|thread| thread.id == id)
    }

    pub const fn current(&self) -> Option<CoreThreadId> {
        self.current
    }

    pub const fn ready_len(&self) -> usize {
        self.ready_len
    }

    fn allocate_pid(&mut self) -> CoreTaskId {
        let id = CoreTaskId(self.next_pid);
        self.next_pid = self.next_pid.saturating_add(1);
        id
    }

    fn allocate_tid(&mut self) -> CoreThreadId {
        let id = CoreThreadId(self.next_tid);
        self.next_tid = self.next_tid.saturating_add(1);
        id
    }

    fn insert_task(&mut self, task: CoreTask) -> Result<(), CoreMtssError> {
        let mut idx = 0usize;
        while idx < TASKS {
            if self.tasks[idx].is_none() {
                self.tasks[idx] = Some(task);
                return Ok(());
            }
            idx += 1;
        }
        Err(CoreMtssError::TaskTableFull)
    }

    fn insert_thread(&mut self, thread: CoreThread) -> Result<(), CoreMtssError> {
        let mut idx = 0usize;
        while idx < THREADS {
            if self.threads[idx].is_none() {
                self.threads[idx] = Some(thread);
                return Ok(());
            }
            idx += 1;
        }
        Err(CoreMtssError::ThreadTableFull)
    }

    fn enqueue(&mut self, thread: CoreThreadId) -> Result<(), CoreMtssError> {
        if self.ready_len == READY {
            return Err(CoreMtssError::ReadyQueueFull);
        }
        self.ready[self.ready_tail] = Some(thread);
        self.ready_tail = (self.ready_tail + 1) % READY;
        self.ready_len += 1;
        Ok(())
    }

    fn dequeue(&mut self) -> Result<CoreThreadId, CoreMtssError> {
        if self.ready_len == 0 {
            return Err(CoreMtssError::ReadyQueueEmpty);
        }
        let thread = self.ready[self.ready_head]
            .take()
            .ok_or(CoreMtssError::ReadyQueueEmpty)?;
        self.ready_head = (self.ready_head + 1) % READY;
        self.ready_len -= 1;
        Ok(thread)
    }

    fn task_mut(&mut self, id: CoreTaskId) -> Option<&mut CoreTask> {
        self.tasks.iter_mut().flatten().find(|task| task.id == id)
    }

    fn thread_mut(&mut self, id: CoreThreadId) -> Option<&mut CoreThread> {
        self.threads
            .iter_mut()
            .flatten()
            .find(|thread| thread.id == id)
    }
}

pub const fn is_canonical_user(address: u64) -> bool {
    address < 0x0000_8000_0000_0000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_allocation_starts_at_userspace_pid_one() {
        let mut mtss: CoreMtss<4, 4, 4> = CoreMtss::new();
        mtss.init_with_idle(StackRange::new(0x8000, 0x9000))
            .unwrap();
        let pid = mtss
            .spawn_userspace(
                "spider-rs",
                UserProgramImage {
                    entry: 0x401000,
                    address_space: AddressSpaceId::new(42),
                    user_stack: StackRange::new(0x7000_0000, 0x7000_4000),
                    cr3: 0x1000,
                },
                StackRange::new(0x9000, 0xa000),
            )
            .unwrap();
        assert_eq!(pid, CoreTaskId::FIRST_USERSPACE);
        let task = mtss.task(pid).unwrap();
        assert_eq!(task.kind, TaskKind::Userspace);
        assert_eq!(task.state, CoreTaskState::Ready);
    }

    #[test]
    fn scheduler_queue_enqueues_and_dequeues_round_robin() {
        let mut mtss: CoreMtss<4, 4, 4> = CoreMtss::new();
        mtss.init_with_idle(StackRange::new(0x8000, 0x9000))
            .unwrap();
        assert_eq!(mtss.ready_len(), 1);
        assert_eq!(mtss.schedule_next().unwrap(), Some(CoreThreadId::IDLE));
        assert_eq!(mtss.current(), Some(CoreThreadId::IDLE));
    }

    #[test]
    fn rejects_noncanonical_userspace_entry() {
        let mut mtss: CoreMtss<4, 4, 4> = CoreMtss::new();
        let error = mtss
            .spawn_userspace(
                "bad",
                UserProgramImage {
                    entry: 0xffff_8000_0000_0000,
                    address_space: AddressSpaceId::new(1),
                    user_stack: StackRange::new(0x7000_0000, 0x7000_4000),
                    cr3: 0x1000,
                },
                StackRange::new(0x9000, 0xa000),
            )
            .unwrap_err();
        assert_eq!(error, CoreMtssError::InvalidEntry);
    }
}
