#![no_std]
#![forbid(unsafe_code)]

//

pub mod scheduler_modules;
pub use scheduler_modules::{MtssCpuProfile, MtssSchedulerModuleDescriptor, MtssSchedulerModuleId};
//Mirage Multitasking Subsystem (MTSS) primitives.
//
// MTSS defines the portable task/thread lifecycle model used by Mirage
// scheduler-facing code.  Architecture-specific CPU context, selectors,
// syscall traps, and trap-frame layouts intentionally remain outside this
// crate; MTSS only records scheduler-visible identity, state, priority,
// timeslice, run-queue, event, and accounting data.

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod backend {
    //! Placeholder contracts for future MTSS integrations.

    use crate::{
        lifecycle::LifecycleEvent,
        stats::MtssStats,
        types::{CpuId, MtssError, ThreadId, ThreadState, Timeslice, Timestamp},
    };

    /// Portable MTSS CPU backend contract.
    ///
    /// CPU-specific scheduler backend split comes after MTSS exists.
    pub trait MtssBackend {
        fn name(&self) -> &'static str;
        fn init_cpu(&mut self, cpu: CpuId) -> Result<(), MtssError>;
        fn current_cpu(&self) -> CpuId;
        fn read_time_counter(&self) -> u64;
    }

    /// Source of monotonic scheduler time.
    pub trait ClockSource {
        fn now(&self) -> Timestamp;
    }

    /// Mechanism used to arm preemption or accounting timer ticks.
    ///
    /// Real context-switch backend contracts are intentionally left for the
    /// next milestone after the MTSS ownership boundary exists.
    pub trait TimerBackend {
        fn arm_timeslice(&mut self, cpu: CpuId, slice: Timeslice) -> Result<(), MtssError>;
    }

    /// Observer hook for supervisor or test harness lifecycle reporting.
    pub trait LifecycleSink {
        fn record_event(&mut self, event: LifecycleEvent);
    }

    /// Observer hook for exporting scheduler accounting snapshots.
    pub trait StatsSink {
        fn publish_stats(&mut self, stats: MtssStats);
    }

    /// Minimal storage contract for scheduler state backends.
    pub trait ThreadStateStore {
        fn load_state(&self, thread: ThreadId) -> Option<ThreadState>;
        fn store_state(&mut self, thread: ThreadId, state: ThreadState) -> Result<(), MtssError>;
    }
}

pub mod lifecycle {
    //! Lifecycle event types emitted by the MTSS scheduler core.

    use crate::types::{CpuId, TaskId, TaskState, ThreadId, ThreadState, Timestamp};

    /// Stable MTSS event kind consumed by supervisors, test harnesses, or logs.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub enum MtssEventKind {
        TaskCreated,
        ThreadCreated,
        ThreadRunnable,
        ThreadRunning,
        ThreadBlocked,
        ThreadSleeping,
        TaskSuspect,
        TaskContained,
        TaskTerminated,
        TaskReaped,
        ThreadExited,
        TimesliceExpired,
    }

    /// Decoupled MTSS event record.
    ///
    /// The record intentionally carries only portable MTSS identities and
    /// scheduler time, so this crate does not depend on the supervisor, RAMFS,
    /// filesystem services, or kernel internals.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct MtssEvent {
        pub kind: MtssEventKind,
        pub task: Option<TaskId>,
        pub thread: Option<ThreadId>,
        pub cpu: Option<CpuId>,
        pub at: Timestamp,
    }

    impl MtssEvent {
        pub const fn task(kind: MtssEventKind, task: TaskId, at: Timestamp) -> Self {
            Self {
                kind,
                task: Some(task),
                thread: None,
                cpu: None,
                at,
            }
        }

        pub const fn thread(
            kind: MtssEventKind,
            task: TaskId,
            thread: ThreadId,
            cpu: Option<CpuId>,
            at: Timestamp,
        ) -> Self {
            Self {
                kind,
                task: Some(task),
                thread: Some(thread),
                cpu,
                at,
            }
        }
    }

    /// Consumer-side sink for draining MTSS events without coupling MTSS to any
    /// concrete supervisor, filesystem, RAMFS, or kernel logging implementation.
    pub trait MtssEventSink {
        fn record_mtss_event(&mut self, event: MtssEvent);
    }

    /// Reason a task or micro-thread changed lifecycle state.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub enum LifecycleReason {
        Created,
        Admitted,
        Scheduled,
        Yielded,
        Preempted,
        Blocked,
        Sleeping,
        Woken,
        Suspended,
        Suspected,
        Contained,
        Exited,
        Reaped,
        Faulted,
        Revoked,
    }

    /// Scheduler lifecycle event suitable for supervisor recovery logs.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct LifecycleEvent {
        pub task: Option<TaskId>,
        pub thread: Option<ThreadId>,
        pub cpu: Option<CpuId>,
        pub previous_task: Option<TaskState>,
        pub next_task: Option<TaskState>,
        pub previous_thread: Option<ThreadState>,
        pub next_thread: Option<ThreadState>,
        pub reason: LifecycleReason,
        pub at: Timestamp,
    }

    impl LifecycleEvent {
        pub const fn thread(
            thread: ThreadId,
            cpu: Option<CpuId>,
            previous: Option<ThreadState>,
            next: ThreadState,
            reason: LifecycleReason,
            at: Timestamp,
        ) -> Self {
            Self {
                task: None,
                thread: Some(thread),
                cpu,
                previous_task: None,
                next_task: None,
                previous_thread: previous,
                next_thread: Some(next),
                reason,
                at,
            }
        }

        pub const fn task(
            task: TaskId,
            previous: Option<TaskState>,
            next: TaskState,
            reason: LifecycleReason,
            at: Timestamp,
        ) -> Self {
            Self {
                task: Some(task),
                thread: None,
                cpu: None,
                previous_task: previous,
                next_task: Some(next),
                previous_thread: None,
                next_thread: None,
                reason,
                at,
            }
        }
    }
}

pub mod mtss;
pub mod run_queue;
pub mod task_core;

pub mod scheduler {
    //! Small scheduler core building blocks.
    //!
    //! `SchedulerCore` tracks the currently selected thread and accounting
    //! counters. It is not a production queue implementation; concrete policy
    //! belongs in scheduler services built on this crate.

    use crate::{
        lifecycle::{LifecycleEvent, LifecycleReason},
        stats::MtssStats,
        types::{CpuId, MtssError, ThreadId, ThreadState, Timestamp},
    };

    /// Current scheduling decision for a CPU.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct ScheduleDecision {
        pub cpu: CpuId,
        pub previous: Option<ThreadId>,
        pub next: ThreadId,
        pub at: Timestamp,
    }

    impl ScheduleDecision {
        pub const fn new(
            cpu: CpuId,
            previous: Option<ThreadId>,
            next: ThreadId,
            at: Timestamp,
        ) -> Self {
            Self {
                cpu,
                previous,
                next,
                at,
            }
        }
    }

    /// Minimal MTSS scheduler core state.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct SchedulerCore {
        cpu: CpuId,
        current: Option<ThreadId>,
        stats: MtssStats,
    }

    impl SchedulerCore {
        pub const fn new(cpu: CpuId) -> Self {
            Self {
                cpu,
                current: None,
                stats: MtssStats::new(),
            }
        }

        pub const fn cpu(&self) -> CpuId {
            self.cpu
        }
        pub const fn current(&self) -> Option<ThreadId> {
            self.current
        }
        pub const fn stats(&self) -> MtssStats {
            self.stats
        }

        pub fn admit(&mut self, thread: ThreadId, at: Timestamp) -> LifecycleEvent {
            self.stats = self.stats.with_admission();
            LifecycleEvent::thread(
                thread,
                Some(self.cpu),
                Some(ThreadState::New),
                ThreadState::Ready,
                LifecycleReason::Admitted,
                at,
            )
        }

        pub fn select(
            &mut self,
            next: ThreadId,
            at: Timestamp,
        ) -> Result<ScheduleDecision, MtssError> {
            if Some(next) == self.current {
                return Err(MtssError::AlreadyCurrent);
            }
            let previous = self.current;
            self.current = Some(next);
            self.stats = self.stats.with_context_switch();
            Ok(ScheduleDecision::new(self.cpu, previous, next, at))
        }

        pub fn clear_current(
            &mut self,
            reason: LifecycleReason,
            at: Timestamp,
        ) -> Option<LifecycleEvent> {
            let thread = self.current.take()?;
            self.stats = self.stats.with_completion();
            Some(LifecycleEvent::thread(
                thread,
                Some(self.cpu),
                Some(ThreadState::Running),
                ThreadState::Dead,
                reason,
                at,
            ))
        }
    }
}

pub mod stats {
    //! MTSS accounting counters.

    /// Monotonic accounting counters maintained by scheduler cores.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct MtssStats {
        pub admitted_tasks: u64,
        pub completed_tasks: u64,
        pub reaped_tasks: u64,
        pub admitted_threads: u64,
        pub completed_threads: u64,
        pub context_switches: u64,
        pub preemptions: u64,
        pub blocked_tasks: u64,
        pub blocked_threads: u64,
        pub sleeps: u64,
        pub wakeups: u64,
        pub suspensions: u64,
        pub containments: u64,
    }

    impl MtssStats {
        pub const fn new() -> Self {
            Self {
                admitted_tasks: 0,
                completed_tasks: 0,
                reaped_tasks: 0,
                admitted_threads: 0,
                completed_threads: 0,
                context_switches: 0,
                preemptions: 0,
                blocked_tasks: 0,
                blocked_threads: 0,
                sleeps: 0,
                wakeups: 0,
                suspensions: 0,
                containments: 0,
            }
        }

        pub const fn with_task_admission(mut self) -> Self {
            self.admitted_tasks += 1;
            self
        }
        pub const fn with_task_completion(mut self) -> Self {
            self.completed_tasks += 1;
            self
        }
        pub const fn with_reap(mut self) -> Self {
            self.reaped_tasks += 1;
            self
        }
        pub const fn with_admission(mut self) -> Self {
            self.admitted_threads += 1;
            self
        }
        pub const fn with_completion(mut self) -> Self {
            self.completed_threads += 1;
            self
        }
        pub const fn with_context_switch(mut self) -> Self {
            self.context_switches += 1;
            self
        }
        pub const fn with_preemption(mut self) -> Self {
            self.preemptions += 1;
            self
        }
        pub const fn with_task_block(mut self) -> Self {
            self.blocked_tasks += 1;
            self
        }
        pub const fn with_block(mut self) -> Self {
            self.blocked_threads += 1;
            self
        }
        pub const fn with_sleep(mut self) -> Self {
            self.sleeps += 1;
            self
        }
        pub const fn with_wakeup(mut self) -> Self {
            self.wakeups += 1;
            self
        }
        pub const fn with_suspension(mut self) -> Self {
            self.suspensions += 1;
            self
        }
        pub const fn with_containment(mut self) -> Self {
            self.containments += 1;
            self
        }
    }

    /// Backward-compatible name used by existing MTSS integrations.
    pub type SchedulerStats = MtssStats;
}

pub mod types {
    //! Common MTSS identifier, task/thread, priority, and state types.

    /// Stable identifier for a Mirage task.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct TaskId(u64);

    impl TaskId {
        pub const IDLE: Self = Self(0);
        pub const FIRST_USERSPACE: Self = Self(1);

        pub const fn new(raw: u64) -> Self {
            Self(raw)
        }
        pub const fn get(self) -> u64 {
            self.0
        }
        pub const fn raw(&self) -> u64 {
            self.0
        }
    }

    /// Stable identifier for a Mirage address space.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct AddressSpaceId(u64);

    impl AddressSpaceId {
        pub const fn new(raw: u64) -> Self {
            Self(raw)
        }
        pub const fn get(self) -> u64 {
            self.0
        }
        pub const fn raw(&self) -> u64 {
            self.0
        }
    }

    /// Stable identifier for a CPU visible to MTSS.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct CpuId(u32);

    impl CpuId {
        pub const fn new(raw: u32) -> Self {
            Self(raw)
        }
        pub const fn get(self) -> u32 {
            self.0
        }
        pub const fn raw(&self) -> u32 {
            self.0
        }
    }

    /// Stable identifier for a Mirage micro-thread.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct ThreadId(u64);

    impl ThreadId {
        pub const IDLE: Self = Self(0);

        pub const fn new(raw: u64) -> Self {
            Self(raw)
        }
        pub const fn get(self) -> u64 {
            self.0
        }
        pub const fn raw(&self) -> u64 {
            self.0
        }
    }

    /// Stable identifier for a Mirage process.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct ProcessId(u64);

    impl ProcessId {
        pub const fn new(raw: u64) -> Self {
            Self(raw)
        }
        pub const fn get(self) -> u64 {
            self.0
        }
        pub const fn raw(&self) -> u64 {
            self.0
        }
    }

    /// Stable identifier for a run queue visible to MTSS.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct RunQueueId(u32);

    impl RunQueueId {
        pub const fn new(raw: u32) -> Self {
            Self(raw)
        }
        pub const fn get(self) -> u32 {
            self.0
        }
        pub const fn raw(&self) -> u32 {
            self.0
        }
    }

    /// Monotonic timestamp in scheduler-defined ticks.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct Timestamp(u64);

    impl Timestamp {
        pub const fn from_ticks(ticks: u64) -> Self {
            Self(ticks)
        }
        pub const fn ticks(self) -> u64 {
            self.0
        }
    }

    /// Duration of a scheduler time slice in ticks.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct Timeslice(u64);

    impl Timeslice {
        pub const fn from_ticks(ticks: u64) -> Self {
            Self(ticks)
        }
        pub const fn ticks(self) -> u64 {
            self.0
        }
        pub const fn is_expired(self) -> bool {
            self.0 == 0
        }
        pub const fn consume_tick(self) -> Self {
            Self(self.0.saturating_sub(1))
        }
    }

    /// Backward-compatible spelling retained for current integrations.
    pub type TimeSlice = Timeslice;

    /// Scheduler-visible task state.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub enum TaskState {
        Created,
        Runnable,
        Running,
        Blocked,
        Exited,
    }

    impl TaskState {
        pub const fn is_terminal(self) -> bool {
            matches!(self, Self::Exited)
        }

        pub const fn may_schedule(self) -> bool {
            matches!(self, Self::Runnable | Self::Running)
        }
    }

    /// Scheduler-visible micro-thread state.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub enum ThreadState {
        New,
        Ready,
        Running,
        Blocked,
        Sleeping,
        Zombie,
        Dead,
    }

    impl ThreadState {
        pub const fn is_terminal(self) -> bool {
            matches!(self, Self::Zombie | Self::Dead)
        }
        pub const fn may_schedule(self) -> bool {
            matches!(self, Self::Ready | Self::Running)
        }
    }

    /// Scheduler-visible process state.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub enum ProcessState {
        New,
        Ready,
        Running,
        Waiting,
        Zombie,
        Dead,
        Failed,
    }

    impl ProcessState {
        pub const fn is_terminal(self) -> bool {
            matches!(self, Self::Dead | Self::Failed)
        }

        pub const fn may_schedule(self) -> bool {
            matches!(self, Self::Ready | Self::Running)
        }
    }

    /// Minimal priority hint. Policy crates decide how hints are interpreted.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct Priority(u8);

    impl Priority {
        pub const CRITICAL: Self = Self(0);
        pub const HIGH: Self = Self(64);
        pub const NORMAL: Self = Self(128);
        pub const LOW: Self = Self(192);

        pub const fn new(raw: u8) -> Self {
            Self(raw)
        }
        pub const fn get(self) -> u8 {
            self.0
        }
        pub const fn raw(self) -> u8 {
            self.0
        }
    }

    /// Portable MTSS task descriptor.  It intentionally omits credentials,
    /// file tables, signals, syscall ABI state, and architecture context.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct Task {
        pub id: TaskId,
        pub parent: Option<TaskId>,
        pub address_space: AddressSpaceId,
        pub state: TaskState,
        pub priority: Priority,
        pub thread_count: u16,
        pub cpu_time_ticks: u128,
    }

    impl Task {
        pub const fn new(
            id: TaskId,
            parent: Option<TaskId>,
            address_space: AddressSpaceId,
            priority: Priority,
        ) -> Self {
            Self {
                id,
                parent,
                address_space,
                state: TaskState::Created,
                priority,
                thread_count: 0,
                cpu_time_ticks: 0,
            }
        }

        pub fn admit(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Runnable)
        }
        pub fn mark_running(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Running)
        }
        pub fn block(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Blocked)
        }
        pub fn sleep(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Blocked)
        }
        pub fn suspend(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Blocked)
        }
        pub fn suspect(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Blocked)
        }
        pub fn contain(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Blocked)
        }
        pub fn terminate(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Exited)
        }
        pub fn reap(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Exited)
        }
        pub fn wake(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Runnable)
        }

        pub fn transition(&mut self, next: TaskState) -> Result<TaskState, MtssError> {
            if !valid_task_transition(self.state, next) {
                return Err(MtssError::InvalidTaskTransition {
                    from: self.state,
                    to: next,
                });
            }
            let previous = self.state;
            self.state = next;
            Ok(previous)
        }

        pub fn increment_thread_count(&mut self) {
            self.thread_count = self.thread_count.saturating_add(1);
        }
        pub fn decrement_thread_count(&mut self) {
            self.thread_count = self.thread_count.saturating_sub(1);
        }
        pub fn accumulate_cpu_time(&mut self, ticks: u64) {
            self.cpu_time_ticks = self.cpu_time_ticks.saturating_add(ticks as u128);
        }
    }

    /// Portable MTSS thread descriptor.  It intentionally omits CPU register
    /// state, trap frames, TLS selector encoding, signal masks, and syscall ABI
    /// details; those remain in the kernel/arch boundary.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct Thread {
        pub id: ThreadId,
        pub task: TaskId,
        pub state: ThreadState,
        pub priority: Priority,
        pub timeslice: Timeslice,
        pub cpu_time_ticks: u128,
    }

    impl Thread {
        pub const fn new(
            id: ThreadId,
            task: TaskId,
            priority: Priority,
            timeslice: Timeslice,
        ) -> Self {
            Self {
                id,
                task,
                state: ThreadState::New,
                priority,
                timeslice,
                cpu_time_ticks: 0,
            }
        }

        pub fn admit(&mut self) -> Result<ThreadState, MtssError> {
            self.transition(ThreadState::Ready)
        }
        pub fn mark_running(&mut self) -> Result<ThreadState, MtssError> {
            self.transition(ThreadState::Running)
        }
        pub fn mark_ready(&mut self) -> Result<ThreadState, MtssError> {
            self.transition(ThreadState::Ready)
        }
        pub fn block(&mut self) -> Result<ThreadState, MtssError> {
            self.transition(ThreadState::Blocked)
        }
        pub fn sleep(&mut self) -> Result<ThreadState, MtssError> {
            self.transition(ThreadState::Sleeping)
        }
        pub fn wake(&mut self) -> Result<ThreadState, MtssError> {
            self.transition(ThreadState::Ready)
        }
        pub fn terminate(&mut self) -> Result<ThreadState, MtssError> {
            self.transition(ThreadState::Dead)
        }

        pub fn consume_timeslice_tick(&mut self) -> bool {
            self.timeslice = self.timeslice.consume_tick();
            self.timeslice.is_expired()
        }

        pub fn reset_timeslice(&mut self, timeslice: Timeslice) {
            self.timeslice = timeslice;
        }

        pub fn accumulate_cpu_time(&mut self, ticks: u64) {
            self.cpu_time_ticks = self.cpu_time_ticks.saturating_add(ticks as u128);
        }

        pub fn transition(&mut self, next: ThreadState) -> Result<ThreadState, MtssError> {
            if !valid_thread_transition(self.state, next) {
                return Err(MtssError::InvalidThreadTransition {
                    from: self.state,
                    to: next,
                });
            }
            let previous = self.state;
            self.state = next;
            Ok(previous)
        }
    }

    pub const fn valid_task_transition(from: TaskState, to: TaskState) -> bool {
        use TaskState::*;
        match (from, to) {
            (Created, Runnable) | (Created, Exited) => true,
            (Runnable, Running) | (Runnable, Blocked) | (Runnable, Exited) => true,
            (Running, Runnable) | (Running, Blocked) | (Running, Exited) => true,
            (Blocked, Runnable) | (Blocked, Exited) => true,
            (state, next) if state as u8 == next as u8 => true,
            _ => false,
        }
    }

    pub const fn valid_thread_transition(from: ThreadState, to: ThreadState) -> bool {
        use ThreadState::*;
        match (from, to) {
            (New, Ready) | (New, Dead) => true,
            (Ready, Running)
            | (Ready, Blocked)
            | (Ready, Sleeping)
            | (Ready, Zombie)
            | (Ready, Dead) => true,
            (Running, Ready)
            | (Running, Blocked)
            | (Running, Sleeping)
            | (Running, Zombie)
            | (Running, Dead) => true,
            (Blocked, Ready) | (Blocked, Zombie) | (Blocked, Dead) => true,
            (Sleeping, Ready) | (Sleeping, Zombie) | (Sleeping, Dead) => true,
            (Zombie, Dead) | (Dead, Dead) => true,
            (state, next) if state as u8 == next as u8 => true,
            _ => false,
        }
    }

    pub const fn valid_process_transition(from: ProcessState, to: ProcessState) -> bool {
        use ProcessState::*;
        match (from, to) {
            (New, Ready) | (New, Failed) => true,
            (Ready, Running) | (Ready, Waiting) | (Ready, Zombie) | (Ready, Failed) => true,
            (Running, Ready) | (Running, Waiting) | (Running, Zombie) | (Running, Failed) => true,
            (Waiting, Ready) | (Waiting, Zombie) | (Waiting, Failed) => true,
            (Zombie, Dead) => true,
            (state, next) if state as u8 == next as u8 => true,
            _ => false,
        }
    }

    /// Static descriptor used when a supervisor admits a thread to MTSS.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct ThreadDescriptor {
        pub id: ThreadId,
        pub priority: Priority,
        pub initial_state: ThreadState,
        pub budget: Option<Timeslice>,
    }

    impl ThreadDescriptor {
        pub const fn new(
            id: ThreadId,
            priority: Priority,
            initial_state: ThreadState,
            budget: Option<Timeslice>,
        ) -> Self {
            Self {
                id,
                priority,
                initial_state,
                budget,
            }
        }
    }

    /// MTSS operation failures.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub enum MtssError {
        RunQueueFull,
        AlreadyCurrent,
        EmptyRunQueue,
        InvalidTask,
        InvalidThread,
        TaskTableFull,
        ThreadTableFull,
        InvalidTaskTransition { from: TaskState, to: TaskState },
        InvalidThreadTransition { from: ThreadState, to: ThreadState },
        BackendUnavailable,
        CapabilityDenied,
    }
}

pub use backend::{
    ClockSource, LifecycleSink, MtssBackend, StatsSink, ThreadStateStore, TimerBackend,
};
pub use lifecycle::{LifecycleEvent, LifecycleReason, MtssEvent, MtssEventKind, MtssEventSink};
pub use mtss::{
    Mtss, MtssConfig, MtssHandle, DEFAULT_EVENT_QUEUE_DEPTH, DEFAULT_MAX_TASKS,
    DEFAULT_MAX_THREADS, DEFAULT_RUN_QUEUE_DEPTH,
};
pub use run_queue::{MtssThreadScheduleRecord, RunQueue};
pub use scheduler::{ScheduleDecision, SchedulerCore};
pub use stats::{MtssStats, SchedulerStats};
pub use task_core::{
    is_canonical_user, CoreMtss, CoreMtssError, CoreTask, CoreTaskId, CoreTaskState, CoreThread,
    CoreThreadId, CpuContext as MtssCpuContext, SavedRegisters, StackRange, TaskKind,
    UserProgramImage, UserThreadPreflight, DEFAULT_READY_QUEUE_SIZE, DEFAULT_TASK_TABLE_SIZE,
    DEFAULT_THREAD_TABLE_SIZE,
};
pub use types::{
    valid_process_transition, valid_task_transition, valid_thread_transition, AddressSpaceId,
    CpuId, MtssError, Priority, ProcessId, ProcessState, RunQueueId, Task, TaskId, TaskState,
    Thread, ThreadDescriptor, ThreadId, ThreadState, TimeSlice, Timeslice, Timestamp,
};

#[cfg(test)]
mod tests {
    use super::*;

    type TestMtss<const EVENTS: usize = 32> = Mtss<4, 8, 8, EVENTS>;

    const CPU: CpuId = CpuId::new(2);
    const TASK: TaskId = TaskId::new(100);
    const OTHER_TASK: TaskId = TaskId::new(200);
    const THREAD_A: ThreadId = ThreadId::new(10);
    const THREAD_B: ThreadId = ThreadId::new(11);

    fn mtss<const EVENTS: usize>() -> TestMtss<EVENTS> {
        Mtss::new(
            MtssConfig::new(CPU)
                .with_initial_time(Timestamp::from_ticks(7))
                .with_default_timeslice(Timeslice::from_ticks(2)),
        )
    }

    fn create_task<const EVENTS: usize>(mtss: &mut TestMtss<EVENTS>) {
        mtss.create_task(TASK, None, AddressSpaceId::new(1), Priority::NORMAL)
            .unwrap();
    }

    fn create_thread<const EVENTS: usize>(mtss: &mut TestMtss<EVENTS>, thread: ThreadId) {
        mtss.create_thread(TASK, thread, Priority::NORMAL).unwrap();
    }

    fn drain<const EVENTS: usize>(mtss: &mut TestMtss<EVENTS>) {
        while mtss.drain_event().is_some() {}
    }

    fn assert_event(
        event: MtssEvent,
        kind: MtssEventKind,
        task: Option<TaskId>,
        thread: Option<ThreadId>,
        cpu: Option<CpuId>,
        at: u64,
    ) {
        assert_eq!(event.kind, kind);
        assert_eq!(event.task, task);
        assert_eq!(event.thread, thread);
        assert_eq!(event.cpu, cpu);
        assert_eq!(event.at, Timestamp::from_ticks(at));
    }

    #[test]
    fn create_task_records_handle_event_and_stats() {
        let mut mtss = mtss::<8>();

        let handle = mtss
            .create_task(
                TASK,
                Some(OTHER_TASK),
                AddressSpaceId::new(55),
                Priority::HIGH,
            )
            .unwrap();

        assert_eq!(handle, MtssHandle::task(TASK));
        assert_eq!(mtss.stats().admitted_tasks, 1);
        assert_eq!(mtss.pending_events(), 1);
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::TaskCreated,
            Some(TASK),
            None,
            None,
            7,
        );
        assert_eq!(mtss.drain_event(), None);
    }

    #[test]
    fn create_thread_records_handle_and_event_without_enqueueing() {
        let mut mtss = mtss::<8>();
        create_task(&mut mtss);
        drain(&mut mtss);

        let handle = mtss.create_thread(TASK, THREAD_A, Priority::LOW).unwrap();

        assert_eq!(handle, MtssHandle::thread(TASK, THREAD_A));
        assert_eq!(mtss.stats().admitted_threads, 0);
        assert_eq!(mtss.pick_next(), Ok(None));
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::ThreadCreated,
            Some(TASK),
            Some(THREAD_A),
            None,
            7,
        );
    }

    #[test]
    fn enqueue_thread_marks_thread_runnable_and_updates_stats() {
        let mut mtss = mtss::<8>();
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        drain(&mut mtss);

        mtss.enqueue_thread(THREAD_A).unwrap();

        assert_eq!(mtss.stats().admitted_threads, 1);
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::ThreadRunnable,
            Some(TASK),
            Some(THREAD_A),
            Some(CPU),
            7,
        );
    }

    #[test]
    fn pick_next_runnable_thread_dispatches_fifo_record() {
        let mut mtss = mtss::<8>();
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        create_thread(&mut mtss, THREAD_B);
        mtss.enqueue_thread(THREAD_A).unwrap();
        mtss.enqueue_thread(THREAD_B).unwrap();
        drain(&mut mtss);

        let decision = mtss.pick_next().unwrap().unwrap();

        assert_eq!(decision.cpu, CPU);
        assert_eq!(decision.previous, None);
        assert_eq!(decision.next, THREAD_A);
        assert_eq!(decision.at, Timestamp::from_ticks(7));
        assert_eq!(mtss.current(), Some(THREAD_A));
        assert_eq!(mtss.stats().context_switches, 1);
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::ThreadRunning,
            Some(TASK),
            Some(THREAD_A),
            Some(CPU),
            7,
        );
    }

    #[test]
    fn timer_tick_without_expiry_only_accounts_cpu_time() {
        let mut mtss = mtss::<8>();
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        mtss.enqueue_thread(THREAD_A).unwrap();
        mtss.pick_next().unwrap().unwrap();
        drain(&mut mtss);

        assert_eq!(mtss.on_timer_tick(), Ok(None));

        assert_eq!(mtss.current(), Some(THREAD_A));
        assert_eq!(mtss.stats().preemptions, 0);
        assert_eq!(mtss.stats().context_switches, 1);
        assert_eq!(mtss.pending_events(), 0);
    }

    #[test]
    fn timer_tick_expiry_defers_reschedule_while_preemption_disabled() {
        let mut mtss: TestMtss<16> = Mtss::new(
            MtssConfig::new(CPU)
                .with_initial_time(Timestamp::from_ticks(7))
                .with_default_timeslice(Timeslice::from_ticks(1)),
        );
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        create_thread(&mut mtss, THREAD_B);
        mtss.enqueue_thread(THREAD_A).unwrap();
        mtss.enqueue_thread(THREAD_B).unwrap();
        assert_eq!(mtss.pick_next().unwrap().unwrap().next, THREAD_A);
        drain(&mut mtss);

        assert_eq!(mtss.on_timer_tick_with_preemption_disabled(true), Ok(None));

        assert_eq!(mtss.current(), Some(THREAD_A));
        assert!(mtss.need_resched());
        assert_eq!(mtss.stats().preemptions, 1);
        assert_eq!(mtss.stats().context_switches, 1);
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::TimesliceExpired,
            Some(TASK),
            Some(THREAD_A),
            Some(CPU),
            8,
        );

        let decision = mtss.reschedule_if_needed().unwrap().unwrap();

        assert_eq!(decision.previous, Some(THREAD_A));
        assert_eq!(decision.next, THREAD_B);
        assert!(!mtss.need_resched());
        assert_eq!(mtss.stats().context_switches, 2);
    }

    #[test]
    fn timer_tick_expires_timeslice_requeues_current_and_dispatches_next() {
        let mut mtss: TestMtss<16> = Mtss::new(
            MtssConfig::new(CPU)
                .with_initial_time(Timestamp::from_ticks(7))
                .with_default_timeslice(Timeslice::from_ticks(1)),
        );
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        create_thread(&mut mtss, THREAD_B);
        mtss.enqueue_thread(THREAD_A).unwrap();
        mtss.enqueue_thread(THREAD_B).unwrap();
        assert_eq!(mtss.pick_next().unwrap().unwrap().next, THREAD_A);
        drain(&mut mtss);

        let decision = mtss.on_timer_tick().unwrap().unwrap();

        assert_eq!(decision.previous, Some(THREAD_A));
        assert_eq!(decision.next, THREAD_B);
        assert_eq!(decision.at, Timestamp::from_ticks(8));
        assert_eq!(mtss.current(), Some(THREAD_B));
        assert_eq!(mtss.stats().preemptions, 1);
        assert_eq!(mtss.stats().context_switches, 2);
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::TimesliceExpired,
            Some(TASK),
            Some(THREAD_A),
            Some(CPU),
            8,
        );
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::ThreadRunnable,
            Some(TASK),
            Some(THREAD_A),
            Some(CPU),
            8,
        );
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::ThreadRunning,
            Some(TASK),
            Some(THREAD_B),
            Some(CPU),
            8,
        );
    }

    #[test]
    fn yield_requeues_current_thread_behind_waiting_runnable_thread() {
        let mut mtss = mtss::<16>();
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        create_thread(&mut mtss, THREAD_B);
        mtss.enqueue_thread(THREAD_A).unwrap();
        mtss.enqueue_thread(THREAD_B).unwrap();
        assert_eq!(mtss.pick_next().unwrap().unwrap().next, THREAD_A);
        drain(&mut mtss);

        let decision = mtss.yield_current().unwrap().unwrap();

        assert_eq!(decision.previous, Some(THREAD_A));
        assert_eq!(decision.next, THREAD_B);
        assert_eq!(mtss.current(), Some(THREAD_B));
        assert_eq!(mtss.stats().context_switches, 2);
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::ThreadRunnable,
            Some(TASK),
            Some(THREAD_A),
            Some(CPU),
            7,
        );
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::ThreadRunning,
            Some(TASK),
            Some(THREAD_B),
            Some(CPU),
            7,
        );

        assert_eq!(mtss.yield_current().unwrap().unwrap().next, THREAD_A);
    }

    #[test]
    fn block_removes_thread_from_run_queue_and_updates_stats() {
        let mut mtss = mtss::<8>();
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        mtss.enqueue_thread(THREAD_A).unwrap();
        drain(&mut mtss);

        mtss.block_thread(THREAD_A).unwrap();

        assert_eq!(mtss.pick_next(), Ok(None));
        assert_eq!(mtss.current(), None);
        assert_eq!(mtss.stats().blocked_threads, 1);
        assert_eq!(mtss.stats().blocked_tasks, 1);
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::ThreadBlocked,
            Some(TASK),
            Some(THREAD_A),
            Some(CPU),
            7,
        );
    }

    #[test]
    fn wake_requeues_blocked_thread_and_updates_stats() {
        let mut mtss = mtss::<16>();
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        mtss.enqueue_thread(THREAD_A).unwrap();
        mtss.block_thread(THREAD_A).unwrap();
        drain(&mut mtss);

        mtss.wake_thread(THREAD_A).unwrap();

        assert_eq!(mtss.stats().wakeups, 1);
        assert_eq!(mtss.stats().admitted_threads, 2);
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::ThreadRunnable,
            Some(TASK),
            Some(THREAD_A),
            Some(CPU),
            7,
        );
        assert_eq!(mtss.pick_next().unwrap().unwrap().next, THREAD_A);
    }

    #[test]
    fn sleep_transition_removes_thread_until_wake_requeues_it() {
        let mut mtss = mtss::<16>();
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        mtss.enqueue_thread(THREAD_A).unwrap();
        drain(&mut mtss);

        mtss.sleep_thread(THREAD_A).unwrap();

        assert_eq!(mtss.pick_next(), Ok(None));
        assert_eq!(mtss.stats().sleeps, 1);
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::ThreadSleeping,
            Some(TASK),
            Some(THREAD_A),
            Some(CPU),
            7,
        );

        mtss.wake_thread(THREAD_A).unwrap();
        assert_eq!(mtss.stats().wakeups, 1);
        assert_eq!(mtss.pick_next().unwrap().unwrap().next, THREAD_A);
    }

    #[test]
    fn contain_task_transition_removes_all_task_threads_from_run_queue() {
        let mut mtss = mtss::<16>();
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        create_thread(&mut mtss, THREAD_B);
        mtss.enqueue_thread(THREAD_A).unwrap();
        mtss.enqueue_thread(THREAD_B).unwrap();
        drain(&mut mtss);

        mtss.contain_task(TASK).unwrap();

        assert_eq!(mtss.pick_next(), Ok(None));
        assert_eq!(mtss.stats().containments, 1);
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::TaskSuspect,
            Some(TASK),
            None,
            None,
            7,
        );
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::TaskContained,
            Some(TASK),
            None,
            None,
            7,
        );
    }

    #[test]
    fn terminate_task_transition_exits_threads_and_clears_scheduling() {
        let mut mtss = mtss::<16>();
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        create_thread(&mut mtss, THREAD_B);
        mtss.enqueue_thread(THREAD_A).unwrap();
        mtss.enqueue_thread(THREAD_B).unwrap();
        drain(&mut mtss);

        mtss.terminate_task(TASK).unwrap();

        assert_eq!(mtss.pick_next(), Ok(None));
        assert_eq!(mtss.stats().completed_tasks, 1);
        assert_eq!(mtss.stats().completed_threads, 2);
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::ThreadExited,
            Some(TASK),
            Some(THREAD_A),
            Some(CPU),
            7,
        );
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::ThreadExited,
            Some(TASK),
            Some(THREAD_B),
            Some(CPU),
            7,
        );
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::TaskTerminated,
            Some(TASK),
            None,
            None,
            7,
        );
    }

    #[test]
    fn reap_task_transition_releases_task_and_thread_slots() {
        let mut mtss = mtss::<16>();
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        mtss.terminate_task(TASK).unwrap();
        drain(&mut mtss);

        mtss.reap_task(TASK).unwrap();

        assert_eq!(mtss.stats().reaped_tasks, 1);
        assert_event(
            mtss.drain_event().unwrap(),
            MtssEventKind::TaskReaped,
            Some(TASK),
            None,
            None,
            7,
        );
        assert_eq!(
            mtss.create_task(TASK, None, AddressSpaceId::new(1), Priority::NORMAL),
            Ok(MtssHandle::task(TASK)),
        );
    }

    #[test]
    fn invalid_transition_is_denied_without_mutating_stats_or_emitting_events() {
        let mut mtss = mtss::<16>();
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        drain(&mut mtss);
        let before = mtss.stats();

        let err = mtss.block_thread(THREAD_A).unwrap_err();

        assert_eq!(
            err,
            MtssError::InvalidThreadTransition {
                from: ThreadState::New,
                to: ThreadState::Blocked,
            }
        );
        assert_eq!(mtss.stats(), before);
        assert_eq!(mtss.pending_events(), 0);

        let err = mtss.reap_task(TASK).unwrap_err();
        assert_eq!(
            err,
            MtssError::InvalidTaskTransition {
                from: TaskState::Runnable,
                to: TaskState::Exited,
            }
        );
    }

    #[test]
    fn events_emitted_correctly_for_full_task_lifecycle() {
        let mut mtss = mtss::<16>();

        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        mtss.enqueue_thread(THREAD_A).unwrap();
        mtss.pick_next().unwrap().unwrap();
        mtss.block_thread(THREAD_A).unwrap();
        mtss.wake_thread(THREAD_A).unwrap();
        mtss.pick_next().unwrap().unwrap();
        mtss.terminate_task(TASK).unwrap();
        mtss.reap_task(TASK).unwrap();

        let expected = [
            (MtssEventKind::TaskCreated, Some(TASK), None, None),
            (
                MtssEventKind::ThreadCreated,
                Some(TASK),
                Some(THREAD_A),
                None,
            ),
            (
                MtssEventKind::ThreadRunnable,
                Some(TASK),
                Some(THREAD_A),
                Some(CPU),
            ),
            (
                MtssEventKind::ThreadRunning,
                Some(TASK),
                Some(THREAD_A),
                Some(CPU),
            ),
            (
                MtssEventKind::ThreadBlocked,
                Some(TASK),
                Some(THREAD_A),
                Some(CPU),
            ),
            (
                MtssEventKind::ThreadRunnable,
                Some(TASK),
                Some(THREAD_A),
                Some(CPU),
            ),
            (
                MtssEventKind::ThreadRunning,
                Some(TASK),
                Some(THREAD_A),
                Some(CPU),
            ),
            (
                MtssEventKind::ThreadExited,
                Some(TASK),
                Some(THREAD_A),
                Some(CPU),
            ),
            (MtssEventKind::TaskTerminated, Some(TASK), None, None),
            (MtssEventKind::TaskReaped, Some(TASK), None, None),
        ];

        for (kind, task, thread, cpu) in expected {
            assert_event(mtss.drain_event().unwrap(), kind, task, thread, cpu, 7);
        }
        assert_eq!(mtss.drain_event(), None);
    }

    #[test]
    fn stats_updated_correctly_across_representative_operations() {
        let mut mtss: TestMtss<32> = Mtss::new(
            MtssConfig::new(CPU)
                .with_initial_time(Timestamp::from_ticks(0))
                .with_default_timeslice(Timeslice::from_ticks(1)),
        );
        create_task(&mut mtss);
        create_thread(&mut mtss, THREAD_A);
        create_thread(&mut mtss, THREAD_B);
        mtss.enqueue_thread(THREAD_A).unwrap();
        mtss.enqueue_thread(THREAD_B).unwrap();
        mtss.pick_next().unwrap().unwrap();
        mtss.on_timer_tick().unwrap().unwrap();
        mtss.block_thread(THREAD_A).unwrap();
        mtss.wake_thread(THREAD_A).unwrap();
        mtss.sleep_thread(THREAD_A).unwrap();
        mtss.wake_thread(THREAD_A).unwrap();
        mtss.contain_task(TASK).unwrap();
        mtss.terminate_task(TASK).unwrap();
        mtss.reap_task(TASK).unwrap();

        let stats = mtss.stats();
        assert_eq!(stats.admitted_tasks, 1);
        assert_eq!(stats.completed_tasks, 1);
        assert_eq!(stats.reaped_tasks, 1);
        assert_eq!(stats.admitted_threads, 4);
        assert_eq!(stats.completed_threads, 2);
        assert_eq!(stats.context_switches, 2);
        assert_eq!(stats.preemptions, 1);
        assert_eq!(stats.blocked_tasks, 0);
        assert_eq!(stats.blocked_threads, 1);
        assert_eq!(stats.sleeps, 1);
        assert_eq!(stats.wakeups, 2);
        assert_eq!(stats.suspensions, 0);
        assert_eq!(stats.containments, 1);
    }

    #[test]
    fn canonical_state_transition_validators_reject_invalid_edges() {
        assert!(valid_task_transition(
            TaskState::Created,
            TaskState::Runnable
        ));
        assert!(!valid_task_transition(
            TaskState::Created,
            TaskState::Running
        ));
        assert!(!valid_task_transition(
            TaskState::Exited,
            TaskState::Runnable
        ));

        assert!(valid_thread_transition(
            ThreadState::New,
            ThreadState::Ready
        ));
        assert!(valid_thread_transition(
            ThreadState::Zombie,
            ThreadState::Dead
        ));
        assert!(!valid_thread_transition(
            ThreadState::New,
            ThreadState::Running
        ));
        assert!(!valid_thread_transition(
            ThreadState::Dead,
            ThreadState::Ready
        ));

        assert!(valid_process_transition(
            ProcessState::New,
            ProcessState::Ready
        ));
        assert!(valid_process_transition(
            ProcessState::Running,
            ProcessState::Failed
        ));
        assert!(!valid_process_transition(
            ProcessState::New,
            ProcessState::Running
        ));
        assert!(!valid_process_transition(
            ProcessState::Dead,
            ProcessState::Ready
        ));
    }
}
