//! Process and thread execution lifecycle operations.
//!
//! Syscall handlers translate ABI arguments into these kernel-internal task
//! requests.  L2 authorization happens here before creating a task domain or
//! replacing an image, while L1 keeps ownership of scheduler queues, thread
//! contexts, descriptor tables, and address-space bookkeeping.

use crate::kernel::process::{
    ExecRequest, ExitStatus, ProcessControlBlock, ProcessId, ProcessPriority, ProcessState,
};
use crate::kernel::scheduler::ScheduledThread;
use crate::kernel::thread::{CpuContext, ThreadControlBlock, ThreadId};
use crate::kernel::{Kernel, KernelError, KernelResult};
use crate::subkernel::Credentials;

/// Linux-compatible clone bits that Mirage currently models.
pub const CLONE_VM: u64 = 0x0000_0100;
pub const CLONE_FS: u64 = 0x0000_0200;
pub const CLONE_FILES: u64 = 0x0000_0400;
pub const CLONE_SIGHAND: u64 = 0x0000_0800;
pub const CLONE_THREAD: u64 = 0x0001_0000;
pub const CLONE_SETTLS: u64 = 0x0008_0000;

const SUPPORTED_CLONE_FLAGS: u64 =
    CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SETTLS;

/// Kernel-internal spawn request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnTaskRequest {
    pub parent: Option<ProcessId>,
    pub entry_point: u64,
    pub priority: ProcessPriority,
    pub credentials: Credentials,
}

/// Kernel-internal clone request.  The syscall ABI may provide a subset of
/// these values, but lifecycle code always works with explicit semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloneTaskRequest {
    pub caller: ProcessId,
    pub current_thread: Option<ThreadId>,
    pub entry_point: u64,
    pub priority: ProcessPriority,
    pub child_stack: Option<u64>,
    pub tls_base: Option<u64>,
    pub flags: u64,
}

impl CloneTaskRequest {
    pub const fn legacy_thread(
        caller: ProcessId,
        current_thread: Option<ThreadId>,
        entry_point: u64,
        priority: ProcessPriority,
    ) -> Self {
        Self {
            caller,
            current_thread,
            entry_point,
            priority,
            child_stack: None,
            tls_base: None,
            flags: CLONE_VM | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD,
        }
    }

    pub const fn new(
        caller: ProcessId,
        current_thread: Option<ThreadId>,
        entry_point: u64,
        priority: ProcessPriority,
        child_stack: Option<u64>,
        tls_base: Option<u64>,
        flags: u64,
    ) -> Self {
        Self {
            caller,
            current_thread,
            entry_point,
            priority,
            child_stack,
            tls_base,
            flags,
        }
    }

    const fn shares_address_space(self) -> bool {
        (self.flags & CLONE_VM) != 0
    }

    const fn shares_descriptors(self) -> bool {
        (self.flags & CLONE_FILES) != 0
    }

    const fn shares_signal_handlers(self) -> bool {
        (self.flags & CLONE_SIGHAND) != 0
    }

    const fn is_thread_group_clone(self) -> bool {
        (self.flags & CLONE_THREAD) != 0
    }
}

impl<const NPROC: usize, const MSG_DEPTH: usize> Kernel<NPROC, MSG_DEPTH> {
    /// Create a new process task after L2 authorizes domain creation.
    pub fn spawn_task(&mut self, request: SpawnTaskRequest) -> KernelResult<ProcessId> {
        if let Some(parent_pid) = request.parent {
            self.ensure_process_exists(parent_pid)?;
            self.authorize_task_creation(parent_pid, request.credentials)?;
        }

        self.create_process_task(
            request.entry_point,
            request.priority,
            request.parent,
            request.credentials,
            None,
        )
    }

    /// Fork the caller into a new process and preserve POSIX return semantics:
    /// parent receives the child pid, while the child's cloned context returns 0.
    pub fn fork_task(
        &mut self,
        parent: ProcessId,
        current_thread: Option<ThreadId>,
    ) -> KernelResult<ProcessId> {
        self.ensure_process_exists(parent)?;
        let credentials = self.current_credentials(parent)?;
        self.authorize_task_creation(parent, credentials)?;

        let parent_index = self.locate_process(parent)?;
        let parent_pcb = self.process_table[parent_index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?;
        let template = self.fork_context(parent, current_thread)?;
        let child = self.create_process_task(
            template.rip,
            parent_pcb.priority,
            Some(parent),
            credentials,
            Some(template),
        )?;
        if let Some(child_thread) = self.first_thread_for_process(child) {
            self.write_thread_syscall_result(child_thread, 0);
        }
        Ok(child)
    }

    /// Replace a process image after L2 validates the executable request.
    pub fn exec_task(
        &mut self,
        request: ExecRequest,
        current_thread: Option<ThreadId>,
    ) -> KernelResult<()> {
        self.authorize_image_replacement(&request)?;
        let closed = self.process_files_mut(request.caller)?.close_on_exec();
        self.release_description_ids(&closed);
        self.replace_process_image(
            request.caller,
            current_thread,
            request.image.entry_point,
            request.image.stack_pointer,
        )?;
        self.security
            .register_task(request.caller, request.requested_credentials)
            .map_err(KernelError::SecurityViolation)?;
        if let Some(pcb) = self.process_table[self.locate_process(request.caller)?].as_mut() {
            pcb.update_credentials(request.requested_credentials);
        }
        Ok(())
    }

    /// Clone execution state into either the caller's thread group or a new
    /// process, depending on CLONE_THREAD.  TCB, TLS, signal mask, descriptor,
    /// and address-space sharing semantics are recorded explicitly.
    pub fn clone_thread(&mut self, request: CloneTaskRequest) -> KernelResult<ThreadId> {
        self.ensure_process_exists(request.caller)?;
        if (request.flags & !SUPPORTED_CLONE_FLAGS) != 0 {
            return Err(KernelError::InvalidArgument);
        }
        if request.is_thread_group_clone() && !request.shares_address_space() {
            return Err(KernelError::InvalidArgument);
        }
        if request.shares_signal_handlers() && !request.shares_address_space() {
            return Err(KernelError::InvalidArgument);
        }

        let source_context = self.clone_source_context(request)?;
        let created_process = !request.is_thread_group_clone();
        let pid = if created_process {
            let credentials = self.current_credentials(request.caller)?;
            self.authorize_task_creation(request.caller, credentials)?;
            self.create_cloned_process_shell(request, source_context, credentials)?
        } else {
            request.caller
        };

        let thread =
            match self.create_thread_from_context(pid, request.priority, source_context, request) {
                Ok(thread) => thread,
                Err(error) => {
                    if created_process {
                        self.rollback_process_shell(pid);
                    }
                    return Err(error);
                }
            };
        self.scheduler
            .enqueue(ScheduledThread::new(thread, pid, request.priority))
            .map_err(|_| {
                self.rollback_thread_creation(thread);
                if created_process {
                    self.rollback_process_shell(pid);
                }
                KernelError::SchedulerFull
            })?;
        Ok(thread)
    }

    pub fn exit_task(&mut self, pid: ProcessId, status: ExitStatus) {
        self.exit_process(pid, status);
    }

    pub fn wait_task(
        &mut self,
        parent: ProcessId,
        selector: i64,
        status_out: *mut i32,
        options: u64,
    ) -> KernelResult<u64> {
        self.wait_for_child(parent, selector, status_out, options)
    }

    fn authorize_task_creation(
        &self,
        parent: ProcessId,
        requested: Credentials,
    ) -> KernelResult<()> {
        self.security
            .authorize_spawn(parent, requested)
            .map_err(KernelError::SecurityViolation)
    }

    fn authorize_image_replacement(&self, request: &ExecRequest) -> KernelResult<()> {
        self.security
            .authorize_exec(request)
            .map_err(KernelError::SecurityViolation)
    }

    fn current_credentials(&self, pid: ProcessId) -> KernelResult<Credentials> {
        let domain_credentials = self
            .security
            .credentials(pid)
            .map_err(KernelError::SecurityViolation)?;
        let process_index = self.locate_process(pid)?;
        let process_credentials = &self.process_table[process_index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .credentials;
        Ok(Credentials::with_unix_credentials(
            domain_credentials.label(),
            domain_credentials.capabilities(),
            domain_credentials.isolation(),
            process_credentials.uid,
            process_credentials.euid,
            process_credentials.gid,
            process_credentials.egid,
            process_credentials.supplementary_groups(),
            process_credentials.supplementary_group_count(),
        ))
    }

    fn create_process_task(
        &mut self,
        entry_point: u64,
        priority: ProcessPriority,
        parent: Option<ProcessId>,
        creds: Credentials,
        context_template: Option<CpuContext>,
    ) -> KernelResult<ProcessId> {
        let slot = self.find_free_slot().ok_or(KernelError::ProcessTableFull)?;
        let pid = self.allocate_pid();
        let mut pcb = ProcessControlBlock::new(pid, entry_point, priority, parent);
        pcb.update_credentials(creds);
        if let Some(parent_pid) = parent {
            pcb.files = self.inherit_process_file_table(parent_pid)?;
            let parent_index = self.locate_process(parent_pid)?;
            if let Some(parent_pcb) = self.process_table[parent_index].as_ref() {
                pcb.process_group = parent_pcb.process_group;
                pcb.session = parent_pcb.session;
                pcb.signal_actions = parent_pcb.signal_actions;
                pcb.address_space_root = parent_pcb.address_space_root;
            }
        }

        self.security.register_task(pid, creds).map_err(|err| {
            self.release_process_file_table(&mut pcb.files);
            KernelError::SecurityViolation(err)
        })?;

        self.process_table[slot] = Some(pcb);

        let thread_id = match context_template {
            Some(context) => self.create_initial_thread_from_context(pid, priority, context),
            None => self.create_thread(pid, entry_point, priority),
        };
        let thread_id = match thread_id {
            Ok(id) => id,
            Err(err) => {
                if let Some(mut failed) = self.process_table[slot].take() {
                    self.release_process_file_table(&mut failed.files);
                }
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
            if let Some(mut failed) = self.process_table[slot].take() {
                self.release_process_file_table(&mut failed.files);
            }
            self.security.revoke_task(pid);
            return Err(KernelError::SchedulerFull);
        }

        Ok(pid)
    }

    fn create_cloned_process_shell(
        &mut self,
        request: CloneTaskRequest,
        context: CpuContext,
        creds: Credentials,
    ) -> KernelResult<ProcessId> {
        let slot = self.find_free_slot().ok_or(KernelError::ProcessTableFull)?;
        let pid = self.allocate_pid();
        let parent_index = self.locate_process(request.caller)?;
        let parent_pcb = self.process_table[parent_index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?;
        let parent_process_group = parent_pcb.process_group;
        let parent_session = parent_pcb.session;
        let parent_signal_actions = parent_pcb.signal_actions;
        let parent_address_space_root = parent_pcb.address_space_root;
        let mut pcb =
            ProcessControlBlock::new(pid, context.rip, request.priority, Some(request.caller));
        pcb.update_credentials(creds);
        pcb.files = self.inherit_process_file_table(request.caller)?;
        pcb.process_group = parent_process_group;
        pcb.session = parent_session;
        if request.shares_signal_handlers() {
            pcb.signal_actions = parent_signal_actions;
        }
        if request.shares_address_space() {
            pcb.address_space_root = parent_address_space_root;
        }

        self.security.register_task(pid, creds).map_err(|err| {
            self.release_process_file_table(&mut pcb.files);
            KernelError::SecurityViolation(err)
        })?;
        self.process_table[slot] = Some(pcb);
        Ok(pid)
    }

    fn rollback_process_shell(&mut self, pid: ProcessId) {
        if let Ok(index) = self.locate_process(pid) {
            if let Some(mut failed) = self.process_table[index].take() {
                self.release_process_file_table(&mut failed.files);
            }
            self.security.revoke_task(pid);
        }
    }

    fn create_initial_thread_from_context(
        &mut self,
        pid: ProcessId,
        priority: ProcessPriority,
        mut context: CpuContext,
    ) -> KernelResult<ThreadId> {
        let slot = self
            .find_free_thread_slot()
            .ok_or(KernelError::ThreadTableFull)?;
        let id = self.allocate_thread_id();
        if context.rsp == 0 {
            context.rsp = self.allocate_stack_pointer(slot, id);
        }
        let mut tcb = ThreadControlBlock::new(id, pid, context.rip, priority, context.rsp);
        tcb.context = context;
        tcb.thread_group = pid;
        self.thread_table[slot] = Some(tcb);
        self.update_process_thread_count(pid, true);
        Ok(id)
    }

    fn create_thread_from_context(
        &mut self,
        pid: ProcessId,
        priority: ProcessPriority,
        mut context: CpuContext,
        request: CloneTaskRequest,
    ) -> KernelResult<ThreadId> {
        let slot = self
            .find_free_thread_slot()
            .ok_or(KernelError::ThreadTableFull)?;
        let id = self.allocate_thread_id();
        context.rip = request.entry_point;
        context.rax = 0;
        context.rsp = request
            .child_stack
            .unwrap_or_else(|| self.allocate_stack_pointer(slot, id));
        if let Some(tls_base) = request.tls_base {
            context.fs = tls_base;
        }

        let mut tcb = ThreadControlBlock::new(id, pid, context.rip, priority, context.rsp);
        tcb.context = context;
        tcb.signal_mask = self
            .source_signal_mask(request.current_thread)
            .unwrap_or(crate::kernel::process::SignalMask::EMPTY);
        tcb.thread_group = if request.is_thread_group_clone() {
            request.caller
        } else {
            pid
        };
        tcb.tls_base = request.tls_base.unwrap_or(context.fs);
        tcb.shares_address_space = request.shares_address_space();
        tcb.shares_descriptor_table = request.shares_descriptors();
        self.thread_table[slot] = Some(tcb);
        self.update_process_thread_count(pid, true);
        Ok(id)
    }

    fn clone_source_context(&self, request: CloneTaskRequest) -> KernelResult<CpuContext> {
        if let Some(thread) = request.current_thread {
            let index = self.locate_thread(thread)?;
            let tcb = self.thread_table[index].ok_or(KernelError::UnknownThread)?;
            if tcb.process != request.caller {
                return Err(KernelError::UnknownThread);
            }
            Ok(tcb.context)
        } else {
            Ok(CpuContext::new(
                request.entry_point,
                0,
                crate::kernel::thread::PrivilegeMode::User,
            ))
        }
    }

    fn fork_context(
        &self,
        parent: ProcessId,
        current_thread: Option<ThreadId>,
    ) -> KernelResult<CpuContext> {
        if let Some(thread) = current_thread {
            let index = self.locate_thread(thread)?;
            let tcb = self.thread_table[index].ok_or(KernelError::UnknownThread)?;
            if tcb.process != parent {
                return Err(KernelError::UnknownThread);
            }
            Ok(tcb.context)
        } else if let Some(thread) = self.first_thread_for_process(parent) {
            let index = self.locate_thread(thread)?;
            Ok(self.thread_table[index]
                .ok_or(KernelError::UnknownThread)?
                .context)
        } else {
            let parent_index = self.locate_process(parent)?;
            let pcb = self.process_table[parent_index]
                .as_ref()
                .ok_or(KernelError::UnknownProcess)?;
            Ok(CpuContext::new(
                pcb.entry_point,
                0,
                crate::kernel::thread::PrivilegeMode::User,
            ))
        }
    }

    fn source_signal_mask(
        &self,
        thread: Option<ThreadId>,
    ) -> Option<crate::kernel::process::SignalMask> {
        let thread = thread?;
        let index = self.locate_thread(thread).ok()?;
        self.thread_table[index].map(|tcb| tcb.signal_mask)
    }

    fn first_thread_for_process(&self, pid: ProcessId) -> Option<ThreadId> {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(thread) = self.thread_table[idx] {
                if thread.process == pid {
                    return Some(thread.id);
                }
            }
            idx += 1;
        }
        None
    }
}
