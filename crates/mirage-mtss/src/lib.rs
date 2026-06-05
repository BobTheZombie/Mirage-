#![no_std]
#![forbid(unsafe_code)]

//! Mirage Micro-Thread Scheduling Service (MTSS) primitives.
//!
//! MTSS defines scheduler-facing types, lifecycle events, accounting counters,
//! and backend contracts for supervisor-managed scheduling services. The crate
//! intentionally keeps policy out of the kernel path: scheduler implementations
//! may choose queues and admission policy, while Mirage kernel mechanisms remain
//! limited to task switching, time accounting, IPC transport, and capability
//! enforcement.

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod backend {
    //! Placeholder contracts for future MTSS integrations.
    //!
    //! These traits define the boundary between an MTSS scheduler core and the
    //! mechanism providers beneath it. They deliberately avoid specifying policy
    //! such as fairness classes, service priority rules, or admission control.

    use crate::{
        lifecycle::LifecycleEvent,
        stats::SchedulerStats,
        types::{CpuId, SchedulerError, ThreadId, ThreadState, TimeSlice, Timestamp},
    };

    /// Source of monotonic scheduler time.
    pub trait ClockSource {
        fn now(&self) -> Timestamp;
    }

    /// Mechanism used to request a low-level context switch.
    pub trait ContextSwitchBackend {
        fn switch_to(&mut self, thread: ThreadId) -> Result<(), SchedulerError>;
    }

    /// Mechanism used to arm preemption or accounting timer ticks.
    pub trait TimerBackend {
        fn arm_timeslice(&mut self, cpu: CpuId, slice: TimeSlice) -> Result<(), SchedulerError>;
    }

    /// Observer hook for supervisor or test harness lifecycle reporting.
    pub trait LifecycleSink {
        fn record_event(&mut self, event: LifecycleEvent);
    }

    /// Observer hook for exporting scheduler accounting snapshots.
    pub trait StatsSink {
        fn publish_stats(&mut self, stats: SchedulerStats);
    }

    /// Minimal storage contract for scheduler state backends.
    pub trait ThreadStateStore {
        fn load_state(&self, thread: ThreadId) -> Option<ThreadState>;
        fn store_state(
            &mut self,
            thread: ThreadId,
            state: ThreadState,
        ) -> Result<(), SchedulerError>;
    }
}

pub mod lifecycle {
    //! Lifecycle event types emitted by the MTSS scheduler core.

    use crate::types::{CpuId, ThreadId, ThreadState, Timestamp};

    /// Reason a micro-thread entered or left the runnable set.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub enum LifecycleReason {
        Created,
        Admitted,
        Yielded,
        Preempted,
        Blocked,
        Woken,
        Exited,
        Faulted,
        Revoked,
    }

    /// Scheduler lifecycle event suitable for supervisor recovery logs.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct LifecycleEvent {
        pub thread: ThreadId,
        pub cpu: Option<CpuId>,
        pub previous: Option<ThreadState>,
        pub next: ThreadState,
        pub reason: LifecycleReason,
        pub at: Timestamp,
    }

    impl LifecycleEvent {
        pub const fn new(
            thread: ThreadId,
            cpu: Option<CpuId>,
            previous: Option<ThreadState>,
            next: ThreadState,
            reason: LifecycleReason,
            at: Timestamp,
        ) -> Self {
            Self {
                thread,
                cpu,
                previous,
                next,
                reason,
                at,
            }
        }
    }
}

pub mod scheduler {
    //! Small scheduler core building blocks.
    //!
    //! `SchedulerCore` tracks the currently selected thread and accounting
    //! counters. It is not a production queue implementation; concrete policy
    //! belongs in scheduler services built on this crate.

    use crate::{
        lifecycle::{LifecycleEvent, LifecycleReason},
        stats::SchedulerStats,
        types::{CpuId, SchedulerError, ThreadId, ThreadState, Timestamp},
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
        stats: SchedulerStats,
    }

    impl SchedulerCore {
        pub const fn new(cpu: CpuId) -> Self {
            Self {
                cpu,
                current: None,
                stats: SchedulerStats::new(),
            }
        }

        pub const fn cpu(&self) -> CpuId {
            self.cpu
        }

        pub const fn current(&self) -> Option<ThreadId> {
            self.current
        }

        pub const fn stats(&self) -> SchedulerStats {
            self.stats
        }

        pub fn admit(&mut self, thread: ThreadId, at: Timestamp) -> LifecycleEvent {
            self.stats = self.stats.with_admission();
            LifecycleEvent::new(
                thread,
                Some(self.cpu),
                None,
                ThreadState::Runnable,
                LifecycleReason::Admitted,
                at,
            )
        }

        pub fn select(
            &mut self,
            next: ThreadId,
            at: Timestamp,
        ) -> Result<ScheduleDecision, SchedulerError> {
            if Some(next) == self.current {
                return Err(SchedulerError::AlreadyCurrent);
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

            Some(LifecycleEvent::new(
                thread,
                Some(self.cpu),
                Some(ThreadState::Running),
                ThreadState::Exited,
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
    pub struct SchedulerStats {
        pub admitted_threads: u64,
        pub completed_threads: u64,
        pub context_switches: u64,
        pub preemptions: u64,
        pub blocked_threads: u64,
        pub wakeups: u64,
    }

    impl SchedulerStats {
        pub const fn new() -> Self {
            Self {
                admitted_threads: 0,
                completed_threads: 0,
                context_switches: 0,
                preemptions: 0,
                blocked_threads: 0,
                wakeups: 0,
            }
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

        pub const fn with_block(mut self) -> Self {
            self.blocked_threads += 1;
            self
        }

        pub const fn with_wakeup(mut self) -> Self {
            self.wakeups += 1;
            self
        }
    }
}

pub mod types {
    //! Common MTSS identifier and scheduler state types.

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
    pub struct TimeSlice(u64);

    impl TimeSlice {
        pub const fn from_ticks(ticks: u64) -> Self {
            Self(ticks)
        }

        pub const fn ticks(self) -> u64 {
            self.0
        }
    }

    /// Scheduler-visible micro-thread state.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub enum ThreadState {
        Created,
        Runnable,
        Running,
        Blocked,
        Exited,
        Faulted,
        Revoked,
    }

    /// Minimal priority hint. Policy crates decide how hints are interpreted.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct Priority(u8);

    impl Priority {
        pub const fn new(raw: u8) -> Self {
            Self(raw)
        }

        pub const fn get(self) -> u8 {
            self.0
        }
    }

    /// Static descriptor used when a supervisor admits a thread to MTSS.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct ThreadDescriptor {
        pub id: ThreadId,
        pub priority: Priority,
        pub initial_state: ThreadState,
        pub budget: Option<TimeSlice>,
    }

    impl ThreadDescriptor {
        pub const fn new(
            id: ThreadId,
            priority: Priority,
            initial_state: ThreadState,
            budget: Option<TimeSlice>,
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
    pub enum SchedulerError {
        AlreadyCurrent,
        EmptyRunQueue,
        InvalidThread,
        BackendUnavailable,
        CapabilityDenied,
    }
}

pub use backend::{
    ClockSource, ContextSwitchBackend, LifecycleSink, StatsSink, ThreadStateStore, TimerBackend,
};
pub use lifecycle::{LifecycleEvent, LifecycleReason};
pub use scheduler::{ScheduleDecision, SchedulerCore};
pub use stats::SchedulerStats;
pub use types::{
    CpuId, Priority, SchedulerError, ThreadDescriptor, ThreadId, ThreadState, TimeSlice, Timestamp,
};
