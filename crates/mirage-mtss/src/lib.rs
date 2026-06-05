#![no_std]
#![forbid(unsafe_code)]

//! Mirage Micro-Thread Scheduling Service (MTSS) primitives.
//!
//! MTSS defines the portable task/thread lifecycle model used by Mirage
//! scheduler-facing code.  Architecture-specific CPU context, selectors,
//! syscall traps, and trap-frame layouts intentionally remain outside this
//! crate; MTSS only records scheduler-visible identity, state, priority,
//! timeslice, run-queue, event, and accounting data.

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod backend {
    //! Placeholder contracts for future MTSS integrations.

    use crate::{
        lifecycle::LifecycleEvent,
        stats::MtssStats,
        types::{CpuId, MtssError, ThreadId, ThreadState, Timeslice, Timestamp},
    };

    /// Source of monotonic scheduler time.
    pub trait ClockSource {
        fn now(&self) -> Timestamp;
    }

    /// Mechanism used to request a low-level context switch.
    pub trait ContextSwitchBackend {
        fn switch_to(&mut self, thread: ThreadId) -> Result<(), MtssError>;
    }

    /// Mechanism used to arm preemption or accounting timer ticks.
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

    /// Public MTSS lifecycle event name requested by the architecture spec.
    pub type MtssEvent = LifecycleEvent;
}

pub mod mtss;
pub mod run_queue;

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
                Some(ThreadState::Created),
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
        Sleeping,
        Suspended,
        Suspect,
        Contained,
        Terminated,
        Reaped,
    }

    impl TaskState {
        pub const fn is_terminal(self) -> bool {
            matches!(self, Self::Terminated | Self::Reaped)
        }

        pub const fn may_schedule(self) -> bool {
            matches!(self, Self::Runnable | Self::Running)
        }
    }

    /// Scheduler-visible micro-thread state.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub enum ThreadState {
        Created,
        Ready,
        Running,
        Blocked,
        Sleeping,
        Dead,
    }

    impl ThreadState {
        pub const fn is_terminal(self) -> bool {
            matches!(self, Self::Dead)
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
            self.transition(TaskState::Sleeping)
        }
        pub fn suspend(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Suspended)
        }
        pub fn suspect(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Suspect)
        }
        pub fn contain(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Contained)
        }
        pub fn terminate(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Terminated)
        }
        pub fn reap(&mut self) -> Result<TaskState, MtssError> {
            self.transition(TaskState::Reaped)
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
                state: ThreadState::Created,
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
            (Created, Runnable) | (Created, Suspended) | (Created, Terminated) => true,
            (Runnable, Running)
            | (Runnable, Blocked)
            | (Runnable, Sleeping)
            | (Runnable, Suspended)
            | (Runnable, Suspect)
            | (Runnable, Terminated) => true,
            (Running, Runnable)
            | (Running, Blocked)
            | (Running, Sleeping)
            | (Running, Suspended)
            | (Running, Suspect)
            | (Running, Terminated) => true,
            (Blocked, Runnable)
            | (Blocked, Suspended)
            | (Blocked, Suspect)
            | (Blocked, Terminated) => true,
            (Sleeping, Runnable)
            | (Sleeping, Suspended)
            | (Sleeping, Suspect)
            | (Sleeping, Terminated) => true,
            (Suspended, Runnable) | (Suspended, Terminated) => true,
            (Suspect, Runnable) | (Suspect, Contained) | (Suspect, Terminated) => true,
            (Contained, Runnable) | (Contained, Terminated) => true,
            (Terminated, Reaped) => true,
            (state, next) if state as u8 == next as u8 => true,
            _ => false,
        }
    }

    pub const fn valid_thread_transition(from: ThreadState, to: ThreadState) -> bool {
        use ThreadState::*;
        match (from, to) {
            (Created, Ready) | (Created, Dead) => true,
            (Ready, Running) | (Ready, Blocked) | (Ready, Sleeping) | (Ready, Dead) => true,
            (Running, Ready) | (Running, Blocked) | (Running, Sleeping) | (Running, Dead) => true,
            (Blocked, Ready) | (Blocked, Dead) => true,
            (Sleeping, Ready) | (Sleeping, Dead) => true,
            (Dead, Dead) => true,
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
    ClockSource, ContextSwitchBackend, LifecycleSink, StatsSink, ThreadStateStore, TimerBackend,
};
pub use lifecycle::{LifecycleEvent, LifecycleReason, MtssEvent};
pub use mtss::{
    Mtss, MtssConfig, MtssHandle, DEFAULT_MAX_TASKS, DEFAULT_MAX_THREADS, DEFAULT_RUN_QUEUE_DEPTH,
};
pub use run_queue::{MtssThreadScheduleRecord, RunQueue};
pub use scheduler::{ScheduleDecision, SchedulerCore};
pub use stats::{MtssStats, SchedulerStats};
pub use types::{
    AddressSpaceId, CpuId, MtssError, Priority, RunQueueId, Task, TaskId, TaskState, Thread,
    ThreadDescriptor, ThreadId, ThreadState, TimeSlice, Timeslice, Timestamp,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_lifecycle_allows_portable_block_and_wake() {
        let mut task = Task::new(
            TaskId::new(1),
            None,
            AddressSpaceId::new(1),
            Priority::NORMAL,
        );
        assert_eq!(task.admit(), Ok(TaskState::Created));
        assert_eq!(task.mark_running(), Ok(TaskState::Runnable));
        assert_eq!(task.block(), Ok(TaskState::Running));
        assert_eq!(task.wake(), Ok(TaskState::Blocked));
        assert_eq!(task.terminate(), Ok(TaskState::Runnable));
        assert_eq!(task.reap(), Ok(TaskState::Terminated));
    }

    #[test]
    fn thread_lifecycle_uses_required_states() {
        let mut thread = Thread::new(
            ThreadId::new(7),
            TaskId::new(1),
            Priority::NORMAL,
            Timeslice::from_ticks(2),
        );
        assert_eq!(thread.admit(), Ok(ThreadState::Created));
        assert_eq!(thread.mark_running(), Ok(ThreadState::Ready));
        assert_eq!(thread.mark_ready(), Ok(ThreadState::Running));
        assert_eq!(thread.sleep(), Ok(ThreadState::Ready));
        assert_eq!(thread.wake(), Ok(ThreadState::Sleeping));
        assert_eq!(thread.block(), Ok(ThreadState::Ready));
        assert_eq!(thread.terminate(), Ok(ThreadState::Blocked));
    }

    #[test]
    fn mtss_schedules_yields_and_preempts_threads() {
        let mut mtss: Mtss<2, 4, 4> = Mtss::new(
            MtssConfig::new(CpuId::new(0)).with_default_timeslice(Timeslice::from_ticks(1)),
        );
        mtss.create_task(
            TaskId::new(1),
            None,
            AddressSpaceId::new(1),
            Priority::NORMAL,
        )
        .unwrap();
        mtss.create_thread(TaskId::new(1), ThreadId::new(10), Priority::NORMAL)
            .unwrap();
        mtss.create_thread(TaskId::new(1), ThreadId::new(11), Priority::NORMAL)
            .unwrap();

        mtss.enqueue_thread(ThreadId::new(10)).unwrap();
        mtss.enqueue_thread(ThreadId::new(11)).unwrap();

        let first = mtss.pick_next().unwrap().unwrap();
        assert_eq!(first.next, ThreadId::new(10));
        let second = mtss.on_timer_tick().unwrap().unwrap();
        assert_eq!(second.previous, Some(ThreadId::new(10)));
        assert_eq!(second.next, ThreadId::new(11));
        assert_eq!(mtss.stats().preemptions, 1);
    }

    #[test]
    fn mtss_rejects_invalid_state_transitions_without_panicking() {
        let mut mtss: Mtss<1, 1, 1> = Mtss::new(MtssConfig::default());
        mtss.create_task(
            TaskId::new(1),
            None,
            AddressSpaceId::new(1),
            Priority::NORMAL,
        )
        .unwrap();
        mtss.create_thread(TaskId::new(1), ThreadId::new(10), Priority::NORMAL)
            .unwrap();

        let err = mtss.block_thread(ThreadId::new(10)).unwrap_err();
        assert_eq!(
            err,
            MtssError::InvalidThreadTransition {
                from: ThreadState::Created,
                to: ThreadState::Blocked,
            }
        );

        let err = mtss.reap_task(TaskId::new(1)).unwrap_err();
        assert_eq!(
            err,
            MtssError::InvalidTaskTransition {
                from: TaskState::Runnable,
                to: TaskState::Reaped,
            }
        );
    }
}
