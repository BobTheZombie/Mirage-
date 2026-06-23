//! Fixed-capacity MTSS scheduler facade.
//!
//! The facade in this module is intentionally allocation-free by default. It
//! keeps task/thread descriptors in caller-sized arrays and uses the portable
//! run queue from [`crate::run_queue`]. Policy remains outside this crate; MTSS
//! only validates lifecycle transitions, maintains scheduler-visible state, and
//! emits minimal scheduling decisions.

use crate::{
    lifecycle::{LifecycleReason, MtssEvent, MtssEventKind, MtssEventSink},
    run_queue::{MtssThreadScheduleRecord, RunQueue},
    scheduler::ScheduleDecision,
    stats::MtssStats,
    types::{
        AddressSpaceId, CpuId, MtssError, Priority, Task, TaskId, TaskState, Thread, ThreadId,
        ThreadState, Timeslice, Timestamp,
    },
};

/// Default number of task slots retained by [`Mtss`].
pub const DEFAULT_MAX_TASKS: usize = 64;
/// Default number of thread slots retained by [`Mtss`].
pub const DEFAULT_MAX_THREADS: usize = 256;
/// Default number of runnable thread records retained by [`Mtss`].
pub const DEFAULT_RUN_QUEUE_DEPTH: usize = 256;
/// Default number of scheduler events retained by [`Mtss`].
pub const DEFAULT_EVENT_QUEUE_DEPTH: usize = 256;

/// Configuration for an MTSS scheduler instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MtssConfig {
    /// CPU whose run queue is represented by this scheduler instance.
    pub cpu: CpuId,
    /// Initial monotonic scheduler timestamp.
    pub initial_time: Timestamp,
    /// Default time slice assigned to newly-created threads.
    pub default_timeslice: Timeslice,
}

impl MtssConfig {
    /// Build a configuration for one CPU with a default four-tick time slice.
    pub const fn new(cpu: CpuId) -> Self {
        Self {
            cpu,
            initial_time: Timestamp::from_ticks(0),
            default_timeslice: Timeslice::from_ticks(4),
        }
    }

    /// Override the initial scheduler timestamp.
    pub const fn with_initial_time(mut self, initial_time: Timestamp) -> Self {
        self.initial_time = initial_time;
        self
    }

    /// Override the default thread time slice.
    pub const fn with_default_timeslice(mut self, default_timeslice: Timeslice) -> Self {
        self.default_timeslice = default_timeslice;
        self
    }
}

impl Default for MtssConfig {
    fn default() -> Self {
        Self::new(CpuId::new(0))
    }
}

/// Stable handle returned by MTSS create/admission calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MtssHandle {
    pub task: TaskId,
    pub thread: Option<ThreadId>,
}

impl MtssHandle {
    pub const fn task(task: TaskId) -> Self {
        Self { task, thread: None }
    }

    pub const fn thread(task: TaskId, thread: ThreadId) -> Self {
        Self {
            task,
            thread: Some(thread),
        }
    }
}

/// Allocation-free MTSS scheduler state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mtss<
    const MAX_TASKS: usize = DEFAULT_MAX_TASKS,
    const MAX_THREADS: usize = DEFAULT_MAX_THREADS,
    const RUN_QUEUE_DEPTH: usize = DEFAULT_RUN_QUEUE_DEPTH,
    const EVENT_QUEUE_DEPTH: usize = DEFAULT_EVENT_QUEUE_DEPTH,
> {
    config: MtssConfig,
    now: Timestamp,
    current: Option<ThreadId>,
    tasks: [Option<Task>; MAX_TASKS],
    threads: [Option<Thread>; MAX_THREADS],
    run_queue: RunQueue<MtssThreadScheduleRecord<ThreadId, TaskId, Priority>, RUN_QUEUE_DEPTH>,
    stats: MtssStats,
    events: [Option<MtssEvent>; EVENT_QUEUE_DEPTH],
    event_head: usize,
    event_len: usize,
    dropped_events: u64,
}

impl<
        const MAX_TASKS: usize,
        const MAX_THREADS: usize,
        const RUN_QUEUE_DEPTH: usize,
        const EVENT_QUEUE_DEPTH: usize,
    > Mtss<MAX_TASKS, MAX_THREADS, RUN_QUEUE_DEPTH, EVENT_QUEUE_DEPTH>
{
    /// Create a fixed-capacity MTSS scheduler instance.
    pub const fn new(config: MtssConfig) -> Self {
        Self {
            config,
            now: config.initial_time,
            current: None,
            tasks: [None; MAX_TASKS],
            threads: [None; MAX_THREADS],
            run_queue: RunQueue::new(),
            stats: MtssStats::new(),
            events: [None; EVENT_QUEUE_DEPTH],
            event_head: 0,
            event_len: 0,
            dropped_events: 0,
        }
    }

    /// Return the scheduler accounting snapshot.
    pub const fn stats(&self) -> MtssStats {
        self.stats
    }

    /// Return the currently running thread, if any.
    pub const fn current(&self) -> Option<ThreadId> {
        self.current
    }

    /// Return the number of queued MTSS events waiting to be drained.
    pub const fn pending_events(&self) -> usize {
        self.event_len
    }

    /// Return the number of events dropped because the fixed event ring was full.
    pub const fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    /// Drain the oldest MTSS event from the internal fixed-capacity ring.
    pub fn drain_event(&mut self) -> Option<MtssEvent> {
        if self.event_len == 0 {
            return None;
        }
        let event = self.events[self.event_head].take();
        self.event_head = (self.event_head + 1) % EVENT_QUEUE_DEPTH;
        self.event_len -= 1;
        event
    }

    /// Drain all queued events into a caller-provided sink.
    pub fn drain_events_to<S: MtssEventSink + ?Sized>(&mut self, sink: &mut S) -> usize {
        let mut drained = 0usize;
        while let Some(event) = self.drain_event() {
            sink.record_mtss_event(event);
            drained += 1;
        }
        drained
    }

    /// Create and admit a task into the runnable task set.
    pub fn create_task(
        &mut self,
        id: TaskId,
        parent: Option<TaskId>,
        address_space: AddressSpaceId,
        priority: Priority,
    ) -> Result<MtssHandle, MtssError> {
        if self.find_task_index(id).is_some() {
            return Err(MtssError::InvalidTask);
        }
        let slot = self.free_task_slot().ok_or(MtssError::TaskTableFull)?;
        let mut task = Task::new(id, parent, address_space, priority);
        task.admit()?;
        self.tasks[slot] = Some(task);
        self.stats = self.stats.with_task_admission();
        self.emit(MtssEvent::task(MtssEventKind::TaskCreated, id, self.now));
        Ok(MtssHandle::task(id))
    }

    /// Create a thread in an existing runnable task.
    pub fn create_thread(
        &mut self,
        task: TaskId,
        thread: ThreadId,
        priority: Priority,
    ) -> Result<MtssHandle, MtssError> {
        if self.find_thread_index(thread).is_some() {
            return Err(MtssError::InvalidThread);
        }
        let task_index = self.find_task_index(task).ok_or(MtssError::InvalidTask)?;
        let task_state = self.tasks[task_index].ok_or(MtssError::InvalidTask)?.state;
        if !task_state.may_schedule() {
            return Err(MtssError::InvalidTaskTransition {
                from: task_state,
                to: TaskState::Runnable,
            });
        }
        let slot = self.free_thread_slot().ok_or(MtssError::ThreadTableFull)?;
        self.threads[slot] = Some(Thread::new(
            thread,
            task,
            priority,
            self.config.default_timeslice,
        ));
        self.with_task_mut(task, |task| task.increment_thread_count())?;
        self.emit(MtssEvent::thread(
            MtssEventKind::ThreadCreated,
            task,
            thread,
            None,
            self.now,
        ));
        Ok(MtssHandle::thread(task, thread))
    }

    /// Validate a thread transition into `Ready` and append it to the run queue.
    pub fn enqueue_thread(&mut self, thread: ThreadId) -> Result<(), MtssError> {
        self.ensure_run_queue_capacity()?;
        let (record, task) = {
            let thread = self.thread_mut(thread)?;
            let previous = thread.state;
            if previous != ThreadState::Ready {
                thread.transition(ThreadState::Ready)?;
            }
            (Self::schedule_record(*thread), thread.task)
        };
        self.run_queue.enqueue(record)?;
        self.stats = self.stats.with_admission();
        self.emit(MtssEvent::thread(
            MtssEventKind::ThreadRunnable,
            task,
            thread,
            Some(self.config.cpu),
            self.now,
        ));
        Ok(())
    }

    /// Pick the next runnable thread and mark it running.
    pub fn pick_next(&mut self) -> Result<Option<ScheduleDecision>, MtssError> {
        let record = match self.run_queue.next() {
            Some(record) => record,
            None => return Ok(None),
        };
        self.dispatch(record.thread, LifecycleReason::Scheduled)
            .map(Some)
    }

    /// Account one timer tick and preempt the current thread when its slice expires.
    pub fn on_timer_tick(&mut self) -> Result<Option<ScheduleDecision>, MtssError> {
        self.now = Timestamp::from_ticks(self.now.ticks().saturating_add(1));
        let Some(current) = self.current else {
            return self.pick_next();
        };

        let expired = {
            let thread = self.thread_mut(current)?;
            if thread.state != ThreadState::Running {
                return Err(MtssError::InvalidThreadTransition {
                    from: thread.state,
                    to: ThreadState::Running,
                });
            }
            thread.accumulate_cpu_time(1);
            thread.consume_timeslice_tick()
        };
        self.with_task_mut_for_thread(current, |task| task.accumulate_cpu_time(1))?;

        if !expired {
            return Ok(None);
        }

        self.stats = self.stats.with_preemption();
        let task = self.thread(current)?.task;
        self.emit(MtssEvent::thread(
            MtssEventKind::TimesliceExpired,
            task,
            current,
            Some(self.config.cpu),
            self.now,
        ));
        self.yield_current()
    }

    /// Voluntarily yield the current thread and pick another runnable thread.
    pub fn yield_current(&mut self) -> Result<Option<ScheduleDecision>, MtssError> {
        let Some(current) = self.current else {
            return self.pick_next();
        };
        self.ready_current_for_requeue(current, LifecycleReason::Yielded)?;
        self.pick_next()
    }

    /// Return the current thread to the runnable queue without selecting a replacement.
    pub fn requeue_current(&mut self) -> Result<(), MtssError> {
        let Some(current) = self.current.take() else {
            return Ok(());
        };
        self.ready_current_for_requeue(current, LifecycleReason::Yielded)
    }

    /// Move a thread to the blocked state and remove it from scheduling.
    pub fn block_thread(&mut self, thread: ThreadId) -> Result<(), MtssError> {
        {
            let thread = self.thread(thread)?;
            validate_thread_destination(thread.state, ThreadState::Blocked)?;
        }
        self.unschedule_thread(thread);
        let task = {
            let thread = self.thread_mut(thread)?;
            thread.block()?;
            thread.task
        };
        self.refresh_task_wait_state(task, TaskState::Blocked)?;
        self.stats = self.stats.with_block();
        self.emit(MtssEvent::thread(
            MtssEventKind::ThreadBlocked,
            task,
            thread,
            Some(self.config.cpu),
            self.now,
        ));
        Ok(())
    }

    /// Wake a blocked or sleeping thread and enqueue it for execution.
    pub fn wake_thread(&mut self, thread: ThreadId) -> Result<(), MtssError> {
        self.ensure_run_queue_capacity()?;
        let task = {
            let thread = self.thread_mut(thread)?;
            thread.wake()?;
            thread.task
        };
        self.wake_task_if_waiting(task)?;
        self.stats = self.stats.with_wakeup();
        self.enqueue_thread(thread)
    }

    /// Move a thread to the sleeping state and remove it from scheduling.
    pub fn sleep_thread(&mut self, thread: ThreadId) -> Result<(), MtssError> {
        {
            let thread = self.thread(thread)?;
            validate_thread_destination(thread.state, ThreadState::Sleeping)?;
        }
        self.unschedule_thread(thread);
        let task = {
            let thread = self.thread_mut(thread)?;
            thread.sleep()?;
            thread.task
        };
        self.refresh_task_wait_state(task, TaskState::Sleeping)?;
        self.stats = self.stats.with_sleep();
        self.emit(MtssEvent::thread(
            MtssEventKind::ThreadSleeping,
            task,
            thread,
            Some(self.config.cpu),
            self.now,
        ));
        Ok(())
    }

    /// Terminate a task and all of its threads, revoking their scheduling slots.
    pub fn terminate_task(&mut self, task: TaskId) -> Result<(), MtssError> {
        let previous = self.task_mut(task)?.terminate()?;
        let mut idx = 0;
        while idx < MAX_THREADS {
            if let Some(mut thread) = self.threads[idx] {
                if thread.task == task {
                    self.unschedule_thread(thread.id);
                    thread.terminate()?;
                    self.threads[idx] = Some(thread);
                    self.stats = self.stats.with_completion();
                    self.emit(MtssEvent::thread(
                        MtssEventKind::ThreadExited,
                        task,
                        thread.id,
                        Some(self.config.cpu),
                        self.now,
                    ));
                }
            }
            idx += 1;
        }
        if previous != TaskState::Terminated {
            self.stats = self.stats.with_task_completion();
        }
        self.emit(MtssEvent::task(
            MtssEventKind::TaskTerminated,
            task,
            self.now,
        ));
        Ok(())
    }

    /// Mark one thread as exited and remove it from scheduling.
    pub fn exit_thread(&mut self, thread: ThreadId) -> Result<(), MtssError> {
        self.unschedule_thread(thread);
        let task = {
            let thread = self.thread_mut(thread)?;
            thread.terminate()?;
            thread.task
        };
        self.with_task_mut(task, |task| task.decrement_thread_count())?;
        self.stats = self.stats.with_completion();
        self.emit(MtssEvent::thread(
            MtssEventKind::ThreadExited,
            task,
            thread,
            Some(self.config.cpu),
            self.now,
        ));
        Ok(())
    }

    /// Move a suspect task into containment and remove its runnable threads.
    pub fn contain_task(&mut self, task: TaskId) -> Result<(), MtssError> {
        {
            let task = self.task_mut(task)?;
            if task.state != TaskState::Suspect {
                task.suspect()?;
            }
        }
        self.emit(MtssEvent::task(MtssEventKind::TaskSuspect, task, self.now));
        self.task_mut(task)?.contain()?;
        let mut idx = 0;
        while idx < MAX_THREADS {
            if let Some(thread) = self.threads[idx] {
                if thread.task == task {
                    self.unschedule_thread(thread.id);
                }
            }
            idx += 1;
        }
        self.stats = self.stats.with_containment();
        self.emit(MtssEvent::task(
            MtssEventKind::TaskContained,
            task,
            self.now,
        ));
        Ok(())
    }

    /// Reap a terminated task and release its fixed-capacity table entries.
    pub fn reap_task(&mut self, task: TaskId) -> Result<(), MtssError> {
        let task_index = self.find_task_index(task).ok_or(MtssError::InvalidTask)?;
        let task_state = self.tasks[task_index].ok_or(MtssError::InvalidTask)?.state;
        crate::types::valid_task_transition(task_state, TaskState::Reaped)
            .then_some(())
            .ok_or(MtssError::InvalidTaskTransition {
                from: task_state,
                to: TaskState::Reaped,
            })?;

        let mut idx = 0;
        while idx < MAX_THREADS {
            if let Some(thread) = self.threads[idx] {
                if thread.task == task && thread.state != ThreadState::Dead {
                    return Err(MtssError::InvalidThreadTransition {
                        from: thread.state,
                        to: ThreadState::Dead,
                    });
                }
            }
            idx += 1;
        }

        self.tasks[task_index]
            .as_mut()
            .ok_or(MtssError::InvalidTask)?
            .reap()?;

        idx = 0;
        while idx < MAX_THREADS {
            if let Some(thread) = self.threads[idx] {
                if thread.task == task {
                    self.threads[idx] = None;
                }
            }
            idx += 1;
        }
        self.tasks[task_index] = None;
        self.stats = self.stats.with_reap();
        self.emit(MtssEvent::task(MtssEventKind::TaskReaped, task, self.now));
        Ok(())
    }

    fn dispatch(
        &mut self,
        thread: ThreadId,
        _reason: LifecycleReason,
    ) -> Result<ScheduleDecision, MtssError> {
        let previous = self.current;
        let default_timeslice = self.config.default_timeslice;
        let task = {
            let thread = self.thread_mut(thread)?;
            thread.mark_running()?;
            thread.reset_timeslice(default_timeslice);
            thread.task
        };
        self.with_task_mut(task, |task| {
            if task.state == TaskState::Runnable {
                let _ = task.mark_running();
            }
        })?;
        self.current = Some(thread);
        self.stats = self.stats.with_context_switch();
        self.emit(MtssEvent::thread(
            MtssEventKind::ThreadRunning,
            task,
            thread,
            Some(self.config.cpu),
            self.now,
        ));
        Ok(ScheduleDecision::new(
            self.config.cpu,
            previous,
            thread,
            self.now,
        ))
    }

    fn ready_current_for_requeue(
        &mut self,
        thread: ThreadId,
        _reason: LifecycleReason,
    ) -> Result<(), MtssError> {
        self.ensure_run_queue_capacity()?;
        let default_timeslice = self.config.default_timeslice;
        let (record, task) = {
            let thread = self.thread_mut(thread)?;
            thread.mark_ready()?;
            thread.reset_timeslice(default_timeslice);
            (Self::schedule_record(*thread), thread.task)
        };
        self.run_queue.requeue(record)?;
        self.emit(MtssEvent::thread(
            MtssEventKind::ThreadRunnable,
            task,
            thread,
            Some(self.config.cpu),
            self.now,
        ));
        Ok(())
    }

    fn schedule_record(thread: Thread) -> MtssThreadScheduleRecord<ThreadId, TaskId, Priority> {
        MtssThreadScheduleRecord::new(
            thread.id,
            thread.task,
            thread.priority,
            timeslice_budget_u8(thread.timeslice),
        )
    }

    fn refresh_task_wait_state(
        &mut self,
        task: TaskId,
        waiting_state: TaskState,
    ) -> Result<(), MtssError> {
        let has_schedulable = self.any_schedulable_thread(task);
        let task = self.task_mut(task)?;
        if has_schedulable {
            if task.state != TaskState::Runnable && task.state != TaskState::Running {
                task.wake()?;
            }
        } else if task.state != waiting_state {
            task.transition(waiting_state)?;
            if waiting_state == TaskState::Blocked {
                self.stats = self.stats.with_task_block();
            }
        }
        Ok(())
    }

    fn wake_task_if_waiting(&mut self, task: TaskId) -> Result<(), MtssError> {
        let task = self.task_mut(task)?;
        if matches!(task.state, TaskState::Blocked | TaskState::Sleeping) {
            task.wake()?;
        }
        Ok(())
    }

    fn any_schedulable_thread(&self, task: TaskId) -> bool {
        let mut idx = 0;
        while idx < MAX_THREADS {
            if let Some(thread) = self.threads[idx] {
                if thread.task == task && thread.state.may_schedule() {
                    return true;
                }
            }
            idx += 1;
        }
        false
    }

    fn ensure_run_queue_capacity(&self) -> Result<(), MtssError> {
        if self.run_queue.len() == RUN_QUEUE_DEPTH {
            Err(MtssError::RunQueueFull)
        } else {
            Ok(())
        }
    }

    fn emit(&mut self, event: MtssEvent) {
        if EVENT_QUEUE_DEPTH == 0 {
            self.dropped_events = self.dropped_events.saturating_add(1);
            return;
        }

        if self.event_len == EVENT_QUEUE_DEPTH {
            self.events[self.event_head] = Some(event);
            self.event_head = (self.event_head + 1) % EVENT_QUEUE_DEPTH;
            self.dropped_events = self.dropped_events.saturating_add(1);
            return;
        }

        let tail = (self.event_head + self.event_len) % EVENT_QUEUE_DEPTH;
        self.events[tail] = Some(event);
        self.event_len += 1;
    }

    fn unschedule_thread(&mut self, thread: ThreadId) {
        self.run_queue.remove_thread(thread);
        if self.current == Some(thread) {
            self.current = None;
        }
    }

    fn with_task_mut_for_thread(
        &mut self,
        thread: ThreadId,
        f: impl FnOnce(&mut Task),
    ) -> Result<(), MtssError> {
        let task = self.thread(thread)?.task;
        self.with_task_mut(task, f)
    }

    fn with_task_mut(&mut self, task: TaskId, f: impl FnOnce(&mut Task)) -> Result<(), MtssError> {
        let task = self.task_mut(task)?;
        f(task);
        Ok(())
    }

    fn thread(&self, thread: ThreadId) -> Result<Thread, MtssError> {
        let idx = self
            .find_thread_index(thread)
            .ok_or(MtssError::InvalidThread)?;
        self.threads[idx].ok_or(MtssError::InvalidThread)
    }

    fn task_mut(&mut self, task: TaskId) -> Result<&mut Task, MtssError> {
        let idx = self.find_task_index(task).ok_or(MtssError::InvalidTask)?;
        self.tasks[idx].as_mut().ok_or(MtssError::InvalidTask)
    }

    fn thread_mut(&mut self, thread: ThreadId) -> Result<&mut Thread, MtssError> {
        let idx = self
            .find_thread_index(thread)
            .ok_or(MtssError::InvalidThread)?;
        self.threads[idx].as_mut().ok_or(MtssError::InvalidThread)
    }

    fn find_task_index(&self, task: TaskId) -> Option<usize> {
        let mut idx = 0;
        while idx < MAX_TASKS {
            if let Some(entry) = self.tasks[idx] {
                if entry.id == task {
                    return Some(idx);
                }
            }
            idx += 1;
        }
        None
    }

    fn find_thread_index(&self, thread: ThreadId) -> Option<usize> {
        let mut idx = 0;
        while idx < MAX_THREADS {
            if let Some(entry) = self.threads[idx] {
                if entry.id == thread {
                    return Some(idx);
                }
            }
            idx += 1;
        }
        None
    }

    fn free_task_slot(&self) -> Option<usize> {
        let mut idx = 0;
        while idx < MAX_TASKS {
            if self.tasks[idx].is_none() {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    fn free_thread_slot(&self) -> Option<usize> {
        let mut idx = 0;
        while idx < MAX_THREADS {
            if self.threads[idx].is_none() {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }
}

fn validate_thread_destination(from: ThreadState, to: ThreadState) -> Result<(), MtssError> {
    crate::types::valid_thread_transition(from, to)
        .then_some(())
        .ok_or(MtssError::InvalidThreadTransition { from, to })
}

const fn timeslice_budget_u8(timeslice: Timeslice) -> u8 {
    if timeslice.ticks() == 0 {
        1
    } else if timeslice.ticks() > u8::MAX as u64 {
        u8::MAX
    } else {
        timeslice.ticks() as u8
    }
}
