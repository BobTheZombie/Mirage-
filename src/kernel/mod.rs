//! Core kernel primitives: process lifecycle, scheduling, IPC routing, and
//! multi-core orchestration.

pub mod cpu;
pub mod device;
pub mod exec;
pub mod fs;
pub mod ipc;
pub mod memory;
pub mod process;
pub mod scheduler;
pub mod services;
pub mod spawn;
pub mod sync;
pub mod syscall;
pub mod thread;
pub mod time;
pub mod timer;

use crate::arch::x86_64::{self, boot::FramebufferInfo, clock, ThreadRunOutcome};
use crate::kernel::cpu::CpuCoreState;
use crate::kernel::device::{
    DeviceDescriptor, DeviceError as DriverError, DeviceId, DeviceKind, DeviceManager,
    MirageDeviceDescriptor,
};
use crate::kernel::exec::{CloneTaskRequest, SpawnTaskRequest};
use crate::kernel::fs::inode::InodeKind;
use crate::kernel::fs::{
    open_flags_from_libc, permissions_from_libc_mode, syscall_error_code_from_vfs, AccessMode,
    CDirEntry, CStat, DescriptorFlags, DirEntry, FileDescriptionId, FileSystem, FileTable,
    FileTableError, FsCredentials, Path, PathError, QfsFileSystem, VfsError, MAX_PATH_BYTES,
};
use crate::kernel::ipc::{Message, MessagePayload, MessageQueue, MessageQueueError};
use crate::kernel::memory::MemoryProtection;
use crate::kernel::process::{
    ExecImageMetadata, ExecRequest, ExecServiceDaemon, ExecSignatureMetadata, ExecVectorMetadata,
    ExitStatus, ProcessControlBlock, ProcessFileTableError, ProcessGroupId, ProcessId, ProcessPath,
    ProcessPriority, ProcessState, SessionId, SignalAction, SignalMask, MAX_EXEC_ARGS,
    MAX_EXEC_ENVS, MAX_SUPPLEMENTARY_GROUPS, SIGCHLD, SIGKILL, SIGTERM,
};
use crate::kernel::scheduler::{ScheduledThread, Scheduler};
use crate::kernel::services::registry::{
    ServiceId as RegistryServiceId, ServiceRegistry, ServiceRegistryError, MAX_DEVICE_CLAIMS,
    MAX_SERVICE_REGISTRATIONS,
};
use crate::kernel::spawn::{
    dependencies_ready, service_manifest_signature_valid, DefaultServiceStartupReport,
    DependencyStatus, ServiceManifest, ServiceStartupReport, StartupState,
    DEFAULT_STARTUP_MANIFEST,
};
use crate::kernel::syscall::{
    SyscallContext, SyscallErrorCode, SyscallNumber, MIRAGE_SYSCALL_ERROR_BIT,
};
use crate::kernel::thread::{CpuContext, ThreadControlBlock, ThreadId, ThreadState, MAX_THREADS};
use crate::kernel::time::KERNEL_TIME;
use crate::kernel::timer::{TimerError, TimerManager, MAX_PROCESS_TIMERS, MAX_SLEEP_ENTRIES};
use crate::subkernel::{
    Credentials, DeviceSecurity, IsolationError, SecurityClass, SecurityKernel,
};
use core::cmp::min;
use core::ptr::NonNull;

pub const MAX_PROCESSES: usize = 64;
pub const MESSAGE_DEPTH: usize = 16;
pub const MAX_DEVICES: usize = 8;
pub const MAX_OPEN_FILES: usize = 64;

const AT_FDCWD: i32 = -100;
const SEEK_SET: u64 = 0;
const SEEK_CUR: u64 = 1;
const SEEK_END: u64 = 2;
const AT_REMOVEDIR: u64 = 0x200;
const AT_SYMLINK_FOLLOW: u64 = 0x400;
const RENAME_NOREPLACE: u64 = 1;
const F_DUPFD: u64 = 0;
const F_GETFD: u64 = 1;
const F_SETFD: u64 = 2;
const F_GETFL: u64 = 3;
const F_DUPFD_CLOEXEC: u64 = 1030;
const FD_CLOEXEC: u64 = 1;
const O_CLOEXEC_RAW: u64 = 0o02000000;

const DEFAULT_ROOT_FILESYSTEM: &[u8] = b"qfs";

struct KernelPathBuf {
    bytes: [u8; MAX_PATH_BYTES],
    len: usize,
}

impl KernelPathBuf {
    fn from_str(raw: &str) -> KernelResult<Self> {
        let mut bytes = [0u8; MAX_PATH_BYTES];
        if raw.is_empty() || raw.len() > MAX_PATH_BYTES {
            return Err(KernelError::Filesystem(VfsError::InvalidPath(
                PathError::TooLong,
            )));
        }
        bytes[..raw.len()].copy_from_slice(raw.as_bytes());
        Ok(Self {
            bytes,
            len: raw.len(),
        })
    }

    fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len]) }
    }

    fn as_path(&self) -> KernelResult<Path<'_>> {
        Path::new(self.as_str()).map_err(map_path_error)
    }

    fn truncate_to_root(&mut self, root_len: usize) {
        self.len = root_len.max(1);
    }

    fn push_component(&mut self, component: &str) -> KernelResult<()> {
        if component.is_empty() || component == "." {
            return Ok(());
        }
        if component == ".." {
            while self.len > 1 && self.bytes[self.len - 1] == b'/' {
                self.len -= 1;
            }
            while self.len > 1 && self.bytes[self.len - 1] != b'/' {
                self.len -= 1;
            }
            if self.len > 1 {
                self.len -= 1;
            }
            return Ok(());
        }
        if component.len() > crate::kernel::fs::MAX_COMPONENT_BYTES {
            return Err(KernelError::Filesystem(VfsError::NameTooLong));
        }
        let extra = if self.len == 1 {
            component.len()
        } else {
            component.len() + 1
        };
        if self.len + extra > MAX_PATH_BYTES {
            return Err(KernelError::Filesystem(VfsError::InvalidPath(
                PathError::TooLong,
            )));
        }
        if self.len != 1 {
            self.bytes[self.len] = b'/';
            self.len += 1;
        }
        self.bytes[self.len..self.len + component.len()].copy_from_slice(component.as_bytes());
        self.len += component.len();
        Ok(())
    }
}

fn is_supported_root_filesystem(filesystem_type: &[u8]) -> bool {
    matches!(filesystem_type, b"qfs" | b"ext4" | b"ssd_usb" | b"ssd-usb")
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirageTimespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirageItimerspec {
    pub it_interval: MirageTimespec,
    pub it_value: MirageTimespec,
}

#[derive(Debug, Clone, Copy)]
pub enum KernelError {
    ProcessTableFull,
    SchedulerFull,
    UnknownProcess,
    UnknownThread,
    ThreadTableFull,
    MessageQueueFull,
    MessageQueueEmpty,
    SecurityViolation(IsolationError),
    IsolationFault(IsolationError),
    DeviceNotFound,
    DeviceFault(DriverError),
    InvalidSyscall,
    InvalidArgument,
    InvalidPointer,
    AllocationFailed,
    FileTableFull,
    Filesystem(VfsError),
}

pub type KernelResult<T> = core::result::Result<T, KernelError>;

const EMPTY_DEVICE_DESCRIPTOR: DeviceDescriptor = DeviceDescriptor::new(
    DeviceId::new(0),
    DeviceKind::SerialConsole,
    "",
    DeviceSecurity::new(SecurityClass::Public, false),
);

pub struct Kernel<const MAX_PROC: usize, const MSG_DEPTH: usize> {
    process_table: [Option<ProcessControlBlock<MAX_OPEN_FILES>>; MAX_PROC],
    ipc_queues: [MessageQueue<MSG_DEPTH>; MAX_PROC],
    scheduler: Scheduler<MAX_THREADS>,
    security: SecurityKernel<MAX_PROC>,
    devices: DeviceManager<MAX_DEVICES>,
    service_registry: ServiceRegistry<MAX_SERVICE_REGISTRATIONS, MAX_DEVICE_CLAIMS>,
    root_fs: QfsFileSystem,
    open_files: FileTable<MAX_OPEN_FILES>,
    core_states: [CpuCoreState; cpu::MAX_CORES],
    thread_table: [Option<ThreadControlBlock>; MAX_THREADS],
    timers: TimerManager<MAX_SLEEP_ENTRIES, MAX_PROCESS_TIMERS>,
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
            devices: DeviceManager::new(),
            service_registry: ServiceRegistry::new(),
            root_fs: QfsFileSystem::new_on_block_device(
                false,
                crate::kernel::device::built_in_block_storage(),
            ),
            open_files: FileTable::new(),
            core_states: [CpuCoreState::new(); cpu::MAX_CORES],
            thread_table: [None; MAX_THREADS],
            timers: TimerManager::new(),
            next_pid: 1,
            next_thread: 1,
            message_sequence: 0,
        }
    }

    pub fn bootstrap(&mut self) {
        self.bootstrap_with_framebuffer(None);
    }

    pub fn bootstrap_with_framebuffer(&mut self, framebuffer: Option<FramebufferInfo>) {
        self.scheduler.reset();
        self.security.reset();
        self.devices.reset();
        self.service_registry.reset();
        self.open_files.clear();
        self.timers.reset();
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

        self.devices
            .install_core_devices_with_framebuffer(framebuffer);
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
        self.spawn_task(SpawnTaskRequest {
            parent: None,
            entry_point: 0,
            priority: ProcessPriority::Critical,
            credentials: creds,
        })
    }

    pub fn bootstrap_services(&mut self) -> DefaultServiceStartupReport {
        self.spawn_services(&DEFAULT_STARTUP_MANIFEST)
    }

    pub fn spawn_services<const CAP: usize>(
        &mut self,
        manifest: &ServiceManifest<CAP>,
    ) -> ServiceStartupReport<CAP> {
        let mut report = ServiceStartupReport::from_manifest(manifest);

        loop {
            let mut made_progress = false;
            let mut pending = 0usize;
            let mut idx = 0usize;

            while idx < report.len() {
                if let Some(record) = report.record(idx) {
                    if record.state == StartupState::Pending {
                        match dependencies_ready(record.descriptor, &report) {
                            DependencyStatus::Ready(parent) => {
                                if !service_manifest_signature_valid(record.descriptor) {
                                    report.set_failed(
                                        idx,
                                        KernelError::SecurityViolation(
                                            IsolationError::PolicyViolation,
                                        ),
                                    );
                                    made_progress = true;
                                    idx += 1;
                                    continue;
                                }

                                report.set_starting(idx);
                                let spawned = if let Some(parent_pid) = parent {
                                    self.spawn_child_process(
                                        parent_pid,
                                        record.descriptor.entry_point,
                                        record.descriptor.priority,
                                        record.descriptor.credentials,
                                    )
                                } else {
                                    self.spawn_initial_process(record.descriptor.credentials)
                                };

                                match spawned {
                                    Ok(pid) => {
                                        report.set_running(idx, pid);
                                        if let Some(registry_service) =
                                            startup_service_to_registry(record.descriptor.id)
                                        {
                                            if let Some(authorizer) = parent {
                                                let _ = self.register_service(
                                                    authorizer,
                                                    registry_service,
                                                    pid,
                                                );
                                            }
                                        }
                                    }
                                    Err(error) => report.set_failed(idx, error),
                                }
                                made_progress = true;
                            }
                            DependencyStatus::Waiting => {
                                pending += 1;
                            }
                            DependencyStatus::Failed => {
                                report.set_failed(idx, KernelError::InvalidArgument);
                                made_progress = true;
                            }
                        }
                    }
                }
                idx += 1;
            }

            if pending == 0 {
                break;
            }

            if !made_progress {
                let mut fail_idx = 0usize;
                while fail_idx < report.len() {
                    if let Some(record) = report.record(fail_idx) {
                        if record.state == StartupState::Pending {
                            report.set_failed(fail_idx, KernelError::InvalidArgument);
                        }
                    }
                    fail_idx += 1;
                }
                break;
            }
        }

        report
    }

    pub fn spawn_child_process(
        &mut self,
        parent_pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
        requested_creds: Credentials,
    ) -> KernelResult<ProcessId> {
        self.spawn_task(SpawnTaskRequest {
            parent: Some(parent_pid),
            entry_point,
            priority,
            credentials: requested_creds,
        })
    }

    pub fn spawn_thread(
        &mut self,
        pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
    ) -> KernelResult<ThreadId> {
        self.clone_thread(CloneTaskRequest::legacy_thread(
            pid,
            None,
            entry_point,
            priority,
        ))
    }

    pub fn terminate_process(&mut self, pid: ProcessId) {
        self.exit_process(pid, ExitStatus::signaled(SIGTERM));
    }

    pub fn exit_process(&mut self, pid: ProcessId, status: ExitStatus) {
        if let Ok(index) = self.locate_process(pid) {
            if let Some(pcb) = self.process_table[index].as_ref() {
                if pcb.state == ProcessState::Zombie {
                    return;
                }
            }
            if let Some(mut pcb) = self.process_table[index].take() {
                self.release_process_file_table(&mut pcb.files);
                pcb.mark_zombie(status);
                self.process_table[index] = Some(pcb);
            }
            self.ipc_queues[index].clear();
            self.scheduler.remove_process(pid);
            self.remove_threads_for_process(pid);
            memory::release_process(pid);
            self.security.revoke_task(pid);
            self.service_registry.revoke_owner(pid);
            self.timers.release_process(pid);
            let _ = self.queue_signal_to_parent(pid, SIGCHLD);
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

    pub fn register_service(
        &mut self,
        authorizer: ProcessId,
        service: RegistryServiceId,
        owner: ProcessId,
    ) -> KernelResult<()> {
        self.security
            .authorize_service_control(authorizer)
            .map_err(KernelError::SecurityViolation)?;
        self.security
            .authorize_service_registration(owner, service.security_class())
            .map_err(KernelError::SecurityViolation)?;
        self.ensure_process_exists(owner)?;
        self.service_registry
            .register(service, owner)
            .map_err(map_service_registry_error)
    }

    pub fn service_owner(&self, service: RegistryServiceId) -> Option<ProcessId> {
        self.service_registry.owner(service)
    }

    pub fn send_service_message(
        &mut self,
        sender: ProcessId,
        service: RegistryServiceId,
        payload: MessagePayload,
    ) -> KernelResult<()> {
        let receiver = self
            .service_registry
            .owner(service)
            .ok_or(KernelError::UnknownProcess)?;
        self.send_message(sender, receiver, payload)
    }

    pub fn claim_service_device(
        &mut self,
        owner: ProcessId,
        service: RegistryServiceId,
        device: DeviceId,
    ) -> KernelResult<()> {
        let descriptor = self
            .devices
            .descriptor(device)
            .ok_or(KernelError::DeviceNotFound)?;
        self.security
            .authorize_device_access(owner, descriptor.security)
            .map_err(KernelError::SecurityViolation)?;
        self.service_registry
            .claim_device(service, owner, descriptor)
            .map_err(map_service_registry_error)
    }

    pub fn release_service_device(
        &mut self,
        owner: ProcessId,
        service: RegistryServiceId,
        device: DeviceId,
    ) -> KernelResult<()> {
        self.service_registry
            .release_device(service, owner, device)
            .map_err(map_service_registry_error)
    }

    pub fn send_message(
        &mut self,
        sender: ProcessId,
        receiver: ProcessId,
        payload: MessagePayload,
    ) -> KernelResult<()> {
        self.security
            .authorize_ipc(sender, receiver, payload.security_class)
            .map_err(KernelError::SecurityViolation)?;

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
            if let Err(err) = self.make_threads_ready(receiver) {
                // Sending to a blocked process is transactional: if the wakeup cannot be
                // scheduled, the receiver stays blocked and the just-enqueued message is
                // removed so callers can retry without duplicating delivery.
                if let Some(pcb) = self.process_table[queue_index].as_mut() {
                    pcb.state = ProcessState::Blocked;
                }
                let _ = self.ipc_queues[queue_index].rollback_last_push();
                return Err(err);
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

    pub fn receive_or_block(&mut self, pid: ProcessId) -> KernelResult<Option<Message>> {
        let queue_index = self.locate_process(pid)?;
        if let Some(message) = self.ipc_queues[queue_index].pop() {
            return Ok(Some(message));
        }

        self.block_process_at_index(pid, queue_index);
        Ok(None)
    }

    pub fn block_for_message(&mut self, pid: ProcessId) {
        if let Ok(index) = self.locate_process(pid) {
            self.block_process_at_index(pid, index);
        }
    }

    pub fn wait(&mut self, parent: ProcessId, status: Option<&mut i32>) -> KernelResult<ProcessId> {
        let status_ptr = status
            .map(|out| out as *mut i32)
            .unwrap_or(core::ptr::null_mut());
        self.wait_for_child(parent, -1, status_ptr, 0)
            .map(ProcessId::new)
    }

    pub fn waitpid(
        &mut self,
        parent: ProcessId,
        pid: i64,
        status: Option<&mut i32>,
        options: u64,
    ) -> KernelResult<ProcessId> {
        let status_ptr = status
            .map(|out| out as *mut i32)
            .unwrap_or(core::ptr::null_mut());
        self.wait_for_child(parent, pid, status_ptr, options)
            .map(ProcessId::new)
    }

    pub fn queue_thread_syscall(
        &mut self,
        thread: ThreadId,
        number: u64,
        args: [u64; syscall::SYSCALL_MAX_ARGS],
    ) -> KernelResult<()> {
        let index = self.locate_thread(thread)?;
        if let Some(tcb) = self.thread_table[index].as_mut() {
            tcb.prepare_syscall(number, args);
            Ok(())
        } else {
            Err(KernelError::UnknownThread)
        }
    }

    pub fn thread_context(&self, thread: ThreadId) -> KernelResult<CpuContext> {
        let index = self.locate_thread(thread)?;
        self.thread_table[index]
            .map(|tcb| tcb.context)
            .ok_or(KernelError::UnknownThread)
    }

    pub fn handle_syscall(&mut self, number: u64, context: SyscallContext) -> KernelResult<u64> {
        match SyscallNumber::from_raw(number).ok_or(KernelError::InvalidSyscall)? {
            SyscallNumber::GetPid => Ok(context.caller.raw()),
            SyscallNumber::Spawn => self.syscall_spawn(context),
            SyscallNumber::SendIpc => self.syscall_send_ipc(context),
            SyscallNumber::ReceiveIpc => self.syscall_receive_ipc(context),
            SyscallNumber::ReceiveOrBlockIpc => self.syscall_receive_or_block_ipc(context),
            SyscallNumber::BlockForIpc => {
                self.security
                    .authorize_ipc_receive(context.caller)
                    .map_err(KernelError::SecurityViolation)?;
                self.block_for_message(context.caller);
                Ok(0)
            }
            SyscallNumber::EnumerateDevices => self.syscall_enumerate_devices(context),
            SyscallNumber::DeviceInfo => self.syscall_device_info(context),
            SyscallNumber::DeviceRead => self.syscall_device_read(context),
            SyscallNumber::DeviceWrite => self.syscall_device_write(context),
            SyscallNumber::Mmap => self.syscall_mmap(context),
            SyscallNumber::Munmap => self.syscall_munmap(context),
            SyscallNumber::Malloc => self.syscall_malloc(context),
            SyscallNumber::Free => self.syscall_free(context),
            SyscallNumber::Realloc => self.syscall_realloc(context),
            SyscallNumber::MallocAligned => self.syscall_malloc_aligned(context),
            SyscallNumber::OpenAt => self.syscall_openat(context),
            SyscallNumber::Close => self.syscall_close(context),
            SyscallNumber::Read => self.syscall_read(context),
            SyscallNumber::Write => self.syscall_write(context),
            SyscallNumber::Pread64 => self.syscall_pread64(context),
            SyscallNumber::Pwrite64 => self.syscall_pwrite64(context),
            SyscallNumber::Lseek => self.syscall_lseek(context),
            SyscallNumber::Statx => self.syscall_statx(context),
            SyscallNumber::NewFstatAt => self.syscall_newfstatat(context),
            SyscallNumber::Getdents64 => self.syscall_getdents64(context),
            SyscallNumber::MkdirAt => self.syscall_mkdirat(context),
            SyscallNumber::UnlinkAt => self.syscall_unlinkat(context),
            SyscallNumber::RenameAt2 => self.syscall_renameat2(context),
            SyscallNumber::Ftruncate => self.syscall_ftruncate(context),
            SyscallNumber::Fsync => self.syscall_fsync(context),
            SyscallNumber::Mount => self.syscall_mount(context),
            SyscallNumber::Chdir => self.syscall_chdir(context),
            SyscallNumber::Fchdir => self.syscall_fchdir(context),
            SyscallNumber::Getcwd => self.syscall_getcwd(context),
            SyscallNumber::Faccessat => self.syscall_faccessat(context),
            SyscallNumber::Fchmodat => self.syscall_fchmodat(context),
            SyscallNumber::Fchownat => self.syscall_fchownat(context),
            SyscallNumber::Symlinkat => self.syscall_symlinkat(context),
            SyscallNumber::Readlinkat => self.syscall_readlinkat(context),
            SyscallNumber::Linkat => self.syscall_linkat(context),
            SyscallNumber::RegisterService => self.syscall_register_service(context),
            SyscallNumber::SendServiceIpc => self.syscall_send_service_ipc(context),
            SyscallNumber::ClaimDevice => self.syscall_claim_device(context),
            SyscallNumber::ReleaseDevice => self.syscall_release_device(context),
            SyscallNumber::Fork => self.syscall_fork(context),
            SyscallNumber::Execve => self.syscall_execve(context),
            SyscallNumber::Exit => self.syscall_exit(context),
            SyscallNumber::Wait4 => self.syscall_wait4(context),
            SyscallNumber::GetPpid => self.syscall_getppid(context),
            SyscallNumber::SetPgid => self.syscall_setpgid(context),
            SyscallNumber::Setsid => self.syscall_setsid(context),
            SyscallNumber::GetUid => self.syscall_getuid(context),
            SyscallNumber::GetEuid => self.syscall_geteuid(context),
            SyscallNumber::SetUid => self.syscall_setuid(context),
            SyscallNumber::GetGid => self.syscall_getgid(context),
            SyscallNumber::SetGid => self.syscall_setgid(context),
            SyscallNumber::GetGroups => self.syscall_getgroups(context),
            SyscallNumber::SetGroups => self.syscall_setgroups(context),
            SyscallNumber::RtSigaction => self.syscall_rt_sigaction(context),
            SyscallNumber::RtSigprocmask => self.syscall_rt_sigprocmask(context),
            SyscallNumber::Kill => self.syscall_kill(context),
            SyscallNumber::RtSigreturn => self.syscall_rt_sigreturn(context),
            SyscallNumber::ClockGettime => self.syscall_clock_gettime(context),
            SyscallNumber::Nanosleep => self.syscall_nanosleep(context),
            SyscallNumber::TimerCreate => self.syscall_timer_create(context),
            SyscallNumber::TimerSettime => self.syscall_timer_settime(context),
            SyscallNumber::TimerGettime => self.syscall_timer_gettime(context),
            SyscallNumber::TimerDelete => self.syscall_timer_delete(context),
            SyscallNumber::Dup => self.syscall_dup(context),
            SyscallNumber::Dup2 => self.syscall_dup2(context),
            SyscallNumber::Dup3 => self.syscall_dup3(context),
            SyscallNumber::Fcntl => self.syscall_fcntl(context),
            SyscallNumber::Ioctl => self.syscall_ioctl(context),
            SyscallNumber::Pipe2 => self.syscall_pipe2(context),
            SyscallNumber::Poll => self.syscall_poll(context),
            SyscallNumber::Pselect => self.syscall_pselect(context),
            SyscallNumber::Eventfd => self.syscall_eventfd(context),
            SyscallNumber::Socket => self.syscall_socket(context),
            SyscallNumber::Bind => self.syscall_bind(context),
            SyscallNumber::Listen => self.syscall_listen(context),
            SyscallNumber::Accept => self.syscall_accept(context),
            SyscallNumber::Connect => self.syscall_connect(context),
            SyscallNumber::Sendmsg => self.syscall_sendmsg(context),
            SyscallNumber::Recvmsg => self.syscall_recvmsg(context),
            SyscallNumber::Clone => self.syscall_clone(context),
            SyscallNumber::Futex => self.syscall_futex(context),
            SyscallNumber::SetThreadArea => self.syscall_set_thread_area(context),
            SyscallNumber::ArchPrctl => self.syscall_arch_prctl(context),
        }
    }

    fn syscall_fork(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.fork_task(context.caller, context.thread)
            .map(|pid| pid.raw())
    }

    fn syscall_execve(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let resolved = self.resolve_user_path(context.caller, AT_FDCWD as u64, context.arg(0))?;
        let path = resolved.as_path()?;
        let stat = self.root_fs.stat(path).map_err(KernelError::Filesystem)?;
        if stat.kind != InodeKind::RegularFile {
            return Err(KernelError::Filesystem(VfsError::PermissionDenied));
        }
        if !crate::kernel::fs::Permissions::new(stat.mode, stat.uid, stat.gid).allows(
            self.fs_credentials_for(context.caller)?,
            AccessMode::Execute,
        ) {
            return Err(KernelError::Filesystem(VfsError::PermissionDenied));
        }

        let entry_point = context.arg(3);
        let stack_pointer = context.arg(4);
        let request = ExecRequest::new(
            context.caller,
            ProcessPath::from_path(path),
            exec_vector_metadata(context.arg(1), MAX_EXEC_ARGS)?,
            exec_vector_metadata(context.arg(2), MAX_EXEC_ENVS)?,
            decode_credentials(context.arg(5))?,
            self.exec_image_metadata(&resolved, stat, entry_point, stack_pointer),
        );

        self.exec_task(request, context.thread)?;
        Ok(0)
    }

    fn syscall_exit(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.exit_task(context.caller, ExitStatus::exited(context.arg(0) as i32));
        Ok(0)
    }

    fn syscall_wait4(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.wait_task(
            context.caller,
            context.arg(0) as i64,
            context.arg(1) as *mut i32,
            context.arg(2),
        )
    }

    fn syscall_getppid(&self, context: SyscallContext) -> KernelResult<u64> {
        let index = self.locate_process(context.caller)?;
        Ok(self.process_table[index]
            .as_ref()
            .and_then(|pcb| pcb.parent)
            .map(|pid| pid.raw())
            .unwrap_or(0))
    }

    fn syscall_setpgid(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let target = if context.arg(0) == 0 {
            context.caller
        } else {
            ProcessId::new(context.arg(0))
        };
        let pgid = if context.arg(1) == 0 {
            ProcessGroupId::new(target.raw())
        } else {
            ProcessGroupId::new(context.arg(1))
        };
        let caller_index = self.locate_process(context.caller)?;
        let caller_session = self.process_table[caller_index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .session;
        let target_index = self.locate_process(target)?;
        let target_pcb = self.process_table[target_index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?;
        if target != context.caller && target_pcb.parent != Some(context.caller) {
            return Err(KernelError::InvalidArgument);
        }
        if target_pcb.session != caller_session {
            return Err(KernelError::InvalidArgument);
        }
        target_pcb.set_process_group(pgid);
        Ok(0)
    }

    fn syscall_setsid(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let index = self.locate_process(context.caller)?;
        let sid = SessionId::new(context.caller.raw());
        let pgid = ProcessGroupId::new(context.caller.raw());
        let pcb = self.process_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?;
        if pcb.process_group.raw() == context.caller.raw() {
            return Err(KernelError::InvalidArgument);
        }
        pcb.set_session(sid);
        pcb.set_process_group(pgid);
        Ok(sid.raw())
    }

    fn syscall_getuid(&self, context: SyscallContext) -> KernelResult<u64> {
        Ok(self.process_credentials(context.caller)?.uid as u64)
    }

    fn syscall_geteuid(&self, context: SyscallContext) -> KernelResult<u64> {
        Ok(self.process_credentials(context.caller)?.euid as u64)
    }

    fn syscall_getgid(&self, context: SyscallContext) -> KernelResult<u64> {
        Ok(self.process_credentials(context.caller)?.gid as u64)
    }

    fn syscall_setuid(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_credential_update(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let uid = u16::try_from(context.arg(0)).map_err(|_| KernelError::InvalidArgument)?;
        self.process_credentials_mut(context.caller)?.set_uid(uid);
        Ok(0)
    }

    fn syscall_setgid(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_credential_update(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let gid = u16::try_from(context.arg(0)).map_err(|_| KernelError::InvalidArgument)?;
        self.process_credentials_mut(context.caller)?.set_gid(gid);
        Ok(0)
    }

    fn syscall_getgroups(&self, context: SyscallContext) -> KernelResult<u64> {
        let credentials = self.process_credentials(context.caller)?;
        let group_count = credentials.supplementary_group_count();
        if context.arg(0) == 0 {
            return Ok(group_count as u64);
        }
        let capacity = context.arg(0) as usize;
        if capacity < group_count {
            return Err(KernelError::InvalidArgument);
        }
        let groups = user_slice_mut_typed::<u32>(context.arg(1), capacity)?;
        let stored = credentials.supplementary_groups();
        let mut idx = 0usize;
        while idx < group_count {
            groups[idx] = stored[idx] as u32;
            idx += 1;
        }
        Ok(group_count as u64)
    }

    fn syscall_setgroups(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_credential_update(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let count = context.arg(0) as usize;
        if count > MAX_SUPPLEMENTARY_GROUPS {
            return Err(KernelError::InvalidArgument);
        }
        let user_groups = user_slice_typed::<u32>(context.arg(1), count)?;
        let mut groups = [0u16; MAX_SUPPLEMENTARY_GROUPS];
        let mut idx = 0usize;
        while idx < count {
            groups[idx] =
                u16::try_from(user_groups[idx]).map_err(|_| KernelError::InvalidArgument)?;
            idx += 1;
        }
        self.process_credentials_mut(context.caller)?
            .set_supplementary_groups(&groups[..count])
            .map_err(|_| KernelError::InvalidArgument)?;
        Ok(0)
    }

    fn syscall_rt_sigaction(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let signal = context.arg(0) as usize;
        if signal == 0 || signal > crate::kernel::process::MAX_SIGNAL_NUMBER {
            return Err(KernelError::InvalidArgument);
        }
        let new_action = context.arg(1) as *const SignalAction;
        let old_action = context.arg(2) as *mut SignalAction;
        let index = self.locate_process(context.caller)?;
        let pcb = self.process_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?;
        if !old_action.is_null() {
            unsafe { old_action.write(pcb.signal_actions[signal]) };
        }
        if !new_action.is_null() {
            pcb.signal_actions[signal] = unsafe { new_action.read() };
        }
        Ok(0)
    }

    fn syscall_rt_sigprocmask(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let how = context.arg(0);
        let new_mask = context.arg(1) as *const SignalMask;
        let old_mask = context.arg(2) as *mut SignalMask;
        let thread = context.thread.ok_or(KernelError::UnknownThread)?;
        let index = self.locate_thread(thread)?;
        let tcb = self.thread_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownThread)?;
        if !old_mask.is_null() {
            unsafe { old_mask.write(tcb.signal_mask) };
        }
        if !new_mask.is_null() {
            let requested = unsafe { new_mask.read() };
            let mask = match how {
                0 => SignalMask::from_bits(tcb.signal_mask.bits() | requested.bits()),
                1 => SignalMask::from_bits(tcb.signal_mask.bits() & !requested.bits()),
                2 => requested,
                _ => return Err(KernelError::InvalidArgument),
            };
            tcb.set_signal_mask(mask);
        }
        Ok(0)
    }

    fn syscall_kill(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.send_signal(context.arg(0) as i64, context.arg(1) as u8)
    }

    fn syscall_rt_sigreturn(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let thread = context.thread.ok_or(KernelError::UnknownThread)?;
        let index = self.locate_thread(thread)?;
        self.thread_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownThread)?
            .finish_signal();
        Ok(0)
    }

    fn syscall_clock_gettime(&self, context: SyscallContext) -> KernelResult<u64> {
        let out = user_out_ptr::<MirageTimespec>(context.arg(1))?;
        let nanos = KERNEL_TIME.now().as_nanos();
        unsafe {
            out.write(MirageTimespec {
                tv_sec: (nanos / 1_000_000_000) as i64,
                tv_nsec: (nanos % 1_000_000_000) as i64,
            });
        }
        Ok(0)
    }

    fn syscall_nanosleep(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let requested = read_user_value::<MirageTimespec>(context.arg(0))?;
        if context.arg(1) != 0 {
            let _ = user_out_ptr::<MirageTimespec>(context.arg(1))?;
        }
        let duration_ns = timespec_to_nanos(requested)?;
        if duration_ns == 0 {
            return Ok(0);
        }

        let wake_deadline = KERNEL_TIME.now().as_nanos().saturating_add(duration_ns);
        self.timers
            .add_sleep(context.caller, context.thread, wake_deadline)
            .map_err(map_timer_error)?;
        let process_index = self.locate_process(context.caller)?;
        self.block_process_at_index(context.caller, process_index);

        if context.arg(1) != 0 {
            write_user_value(
                context.arg(1),
                MirageTimespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                },
            )?;
        }

        Ok(0)
    }

    fn syscall_timer_create(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.ensure_process_exists(context.caller)?;
        let timer_id_out = context.arg(2);
        let _ = user_out_ptr::<u64>(timer_id_out)?;
        let id = self
            .timers
            .create_timer(context.caller)
            .map_err(map_timer_error)?;
        write_user_value(timer_id_out, id)?;
        Ok(0)
    }

    fn syscall_timer_settime(&mut self, context: SyscallContext) -> KernelResult<u64> {
        const TIMER_ABSTIME: u64 = 1;

        let timer_id = context.arg(0);
        let flags = context.arg(1);
        if flags & !TIMER_ABSTIME != 0 {
            return Err(KernelError::InvalidArgument);
        }

        let requested = read_user_value::<MirageItimerspec>(context.arg(2))?;
        let old_value_out = context.arg(3);
        if old_value_out != 0 {
            let _ = user_out_ptr::<MirageItimerspec>(old_value_out)?;
        }
        let value_ns = timespec_to_nanos(requested.it_value)?;
        let interval_ns = timespec_to_nanos(requested.it_interval)?;
        let now_ns = KERNEL_TIME.now().as_nanos();
        let deadline = if value_ns == 0 {
            None
        } else if flags & TIMER_ABSTIME != 0 {
            Some(value_ns)
        } else {
            Some(now_ns.saturating_add(value_ns))
        };

        let previous = self
            .timers
            .set_timer(context.caller, timer_id, deadline, interval_ns)
            .map_err(map_timer_error)?;

        if old_value_out != 0 {
            write_user_value(old_value_out, timer_to_itimerspec(previous, now_ns))?;
        }

        Ok(0)
    }

    fn syscall_timer_gettime(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let _ = user_out_ptr::<MirageItimerspec>(context.arg(1))?;
        let timer = self
            .timers
            .timer(context.caller, context.arg(0))
            .map_err(map_timer_error)?;
        write_user_value(
            context.arg(1),
            timer_to_itimerspec(timer, KERNEL_TIME.now().as_nanos()),
        )?;
        Ok(0)
    }

    fn syscall_timer_delete(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.timers
            .delete_timer(context.caller, context.arg(0))
            .map_err(map_timer_error)?;
        Ok(0)
    }

    fn syscall_dup(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.duplicate_fd(
            context.caller,
            context.arg(0) as usize,
            0,
            DescriptorFlags::EMPTY,
        )
    }

    fn syscall_dup2(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.duplicate_fd_to(
            context.caller,
            context.arg(0) as usize,
            context.arg(1) as usize,
            DescriptorFlags::EMPTY,
            false,
        )
    }

    fn syscall_dup3(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let flags = context.arg(2);
        if flags & !O_CLOEXEC_RAW != 0 || context.arg(0) == context.arg(1) {
            return Err(KernelError::InvalidArgument);
        }
        let descriptor_flags = if flags & O_CLOEXEC_RAW != 0 {
            DescriptorFlags::CLOSE_ON_EXEC
        } else {
            DescriptorFlags::EMPTY
        };
        self.duplicate_fd_to(
            context.caller,
            context.arg(0) as usize,
            context.arg(1) as usize,
            descriptor_flags,
            true,
        )
    }

    fn syscall_fcntl(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let fd = context.arg(0) as usize;
        match context.arg(1) {
            F_DUPFD => self.duplicate_fd(
                context.caller,
                fd,
                context.arg(2) as usize,
                DescriptorFlags::EMPTY,
            ),
            F_DUPFD_CLOEXEC => self.duplicate_fd(
                context.caller,
                fd,
                context.arg(2) as usize,
                DescriptorFlags::CLOSE_ON_EXEC,
            ),
            F_GETFD => Ok(self
                .process_files(context.caller)?
                .get(fd)
                .map_err(map_process_file_table_error)?
                .flags()
                .bits() as u64),
            F_SETFD => {
                let flags = if context.arg(2) & FD_CLOEXEC != 0 {
                    DescriptorFlags::CLOSE_ON_EXEC
                } else {
                    DescriptorFlags::EMPTY
                };
                self.process_files_mut(context.caller)?
                    .get_mut(fd)
                    .map_err(map_process_file_table_error)?
                    .set_flags(flags);
                Ok(0)
            }
            F_GETFL => {
                let description = self.fd_description(context.caller, fd)?;
                Ok(self
                    .open_files
                    .get(description)
                    .map_err(map_file_table_error)?
                    .flags()
                    .bits() as u64)
            }
            _ => Err(KernelError::InvalidArgument),
        }
    }

    fn syscall_ioctl(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_pipe2(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_poll(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_pselect(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_eventfd(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_socket(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_bind(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_listen(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_accept(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_connect(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_sendmsg(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_recvmsg(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_clone(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let entry_point = context.arg(0);
        let priority = decode_priority(context.arg(1))?;
        let flags = context.arg(2);
        let request = if flags == 0 && context.arg(3) == 0 && context.arg(4) == 0 {
            CloneTaskRequest::legacy_thread(context.caller, context.thread, entry_point, priority)
        } else {
            let tls_base = if context.arg(3) == 0 {
                None
            } else {
                Some(context.arg(3))
            };
            let child_stack = if context.arg(4) == 0 {
                None
            } else {
                Some(context.arg(4))
            };
            CloneTaskRequest::new(
                context.caller,
                context.thread,
                entry_point,
                priority,
                child_stack,
                tls_base,
                flags,
            )
        };
        self.clone_thread(request).map(|thread| thread.raw())
    }

    fn syscall_futex(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_set_thread_area(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_arch_prctl(&mut self, _context: SyscallContext) -> KernelResult<u64> {
        Err(KernelError::InvalidSyscall)
    }

    fn syscall_spawn(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let entry_point = context.arg(0);
        let priority = decode_priority(context.arg(1))?;
        let credentials = decode_credentials(context.arg(2))?;
        self.spawn_task(SpawnTaskRequest {
            parent: Some(context.caller),
            entry_point,
            priority,
            credentials,
        })
        .map(|pid| pid.raw())
    }

    fn syscall_send_ipc(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let receiver = ProcessId::new(context.arg(0));
        let data_ptr = context.arg(1) as *const u8;
        let data_len = context.arg(2) as usize;
        let security_class = decode_security_class(context.arg(3))?;
        if data_len > 0 && data_ptr.is_null() {
            return Err(KernelError::InvalidPointer);
        }

        let data = if data_len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(data_ptr, data_len) }
        };
        let payload = MessagePayload::from_slice(security_class, data);
        self.send_message(context.caller, receiver, payload)?;
        Ok(payload.length as u64)
    }

    fn syscall_receive_ipc(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_ipc_receive(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let out = context.arg(0) as *mut Message;
        if out.is_null() {
            return Err(KernelError::InvalidPointer);
        }
        let message = self.receive_message(context.caller)?;
        unsafe { out.write(message) };
        Ok(message.payload.length as u64)
    }

    fn syscall_register_service(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let service = decode_registry_service_id(context.arg(0))?;
        let owner = if context.arg(1) == 0 {
            context.caller
        } else {
            ProcessId::new(context.arg(1))
        };
        self.register_service(context.caller, service, owner)?;
        Ok(0)
    }

    fn syscall_send_service_ipc(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let service = decode_registry_service_id(context.arg(0))?;
        let data_ptr = context.arg(1) as *const u8;
        let data_len = context.arg(2) as usize;
        let security_class = decode_security_class(context.arg(3))?;
        if data_len > 0 && data_ptr.is_null() {
            return Err(KernelError::InvalidPointer);
        }
        let data = if data_len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(data_ptr, data_len) }
        };
        let payload = MessagePayload::from_slice(security_class, data);
        self.send_service_message(context.caller, service, payload)?;
        Ok(payload.length as u64)
    }

    fn syscall_claim_device(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let service = decode_registry_service_id(context.arg(0))?;
        let device = DeviceId::new(context.arg(1) as u16);
        self.claim_service_device(context.caller, service, device)?;
        Ok(0)
    }

    fn syscall_release_device(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let service = decode_registry_service_id(context.arg(0))?;
        let device = DeviceId::new(context.arg(1) as u16);
        self.release_service_device(context.caller, service, device)?;
        Ok(0)
    }

    fn syscall_receive_or_block_ipc(&mut self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_ipc_receive(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let out = context.arg(0) as *mut Message;
        if out.is_null() {
            return Err(KernelError::InvalidPointer);
        }

        if let Some(message) = self.receive_or_block(context.caller)? {
            unsafe { out.write(message) };
            Ok(1)
        } else {
            Ok(0)
        }
    }

    fn syscall_enumerate_devices(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_device_enumeration(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let out = context.arg(0) as *mut MirageDeviceDescriptor;
        let capacity = context.arg(1) as usize;
        if capacity > 0 && out.is_null() {
            return Err(KernelError::InvalidPointer);
        }
        let out_slice = if capacity == 0 {
            &mut []
        } else {
            unsafe { core::slice::from_raw_parts_mut(out, capacity) }
        };

        let mut descriptors = [EMPTY_DEVICE_DESCRIPTOR; MAX_DEVICES];
        let count = self.enumerate_devices(&mut descriptors[..min(capacity, MAX_DEVICES)]);
        let mut idx = 0usize;
        while idx < count {
            out_slice[idx] = MirageDeviceDescriptor::from_descriptor(descriptors[idx]);
            idx += 1;
        }
        Ok(count as u64)
    }

    fn syscall_device_info(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_device_enumeration(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let id = DeviceId::new(context.arg(0) as u16);
        let out = context.arg(1) as *mut MirageDeviceDescriptor;
        if out.is_null() {
            return Err(KernelError::InvalidPointer);
        }
        let descriptor = self.device_info(id).ok_or(KernelError::DeviceNotFound)?;
        unsafe { out.write(MirageDeviceDescriptor::from_descriptor(descriptor)) };
        Ok(1)
    }

    fn syscall_device_read(&self, context: SyscallContext) -> KernelResult<u64> {
        let id = DeviceId::new(context.arg(0) as u16);
        let buffer = context.arg(1) as *mut u8;
        let len = context.arg(2) as usize;
        if len > 0 && buffer.is_null() {
            return Err(KernelError::InvalidPointer);
        }
        let buffer = if len == 0 {
            &mut []
        } else {
            unsafe { core::slice::from_raw_parts_mut(buffer, len) }
        };
        self.device_read(context.caller, id, buffer)
            .map(|read| read as u64)
    }

    fn syscall_device_write(&self, context: SyscallContext) -> KernelResult<u64> {
        let id = DeviceId::new(context.arg(0) as u16);
        let data = context.arg(1) as *const u8;
        let len = context.arg(2) as usize;
        if len > 0 && data.is_null() {
            return Err(KernelError::InvalidPointer);
        }
        let data = if len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(data, len) }
        };
        self.device_write(context.caller, id, data)
            .map(|written| written as u64)
    }

    fn syscall_mmap(&self, context: SyscallContext) -> KernelResult<u64> {
        let length = context.arg(0) as usize;
        let protection = MemoryProtection::from_bits(context.arg(1) as u32);
        self.security
            .authorize_memory_mapping(context.caller, protection)
            .map_err(KernelError::SecurityViolation)?;
        memory::mmap_for(context.caller, length, protection)
            .map(|region| region.as_ptr() as u64)
            .ok_or(KernelError::AllocationFailed)
    }

    fn syscall_munmap(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_memory_service(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let ptr = NonNull::new(context.arg(0) as *mut u8).ok_or(KernelError::InvalidPointer)?;
        let length = context.arg(1) as usize;
        if memory::munmap_ptr_for(context.caller, ptr, length) {
            Ok(0)
        } else {
            Err(KernelError::InvalidArgument)
        }
    }

    fn syscall_malloc(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_memory_service(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        memory::malloc_for(context.caller, context.arg(0) as usize)
            .map(|ptr| ptr.as_ptr() as u64)
            .ok_or(KernelError::AllocationFailed)
    }

    fn syscall_free(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_memory_service(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let ptr = NonNull::new(context.arg(0) as *mut u8).ok_or(KernelError::InvalidPointer)?;
        if memory::free_for(context.caller, ptr) {
            Ok(0)
        } else {
            Err(KernelError::InvalidArgument)
        }
    }

    fn syscall_realloc(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_memory_service(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let ptr = NonNull::new(context.arg(0) as *mut u8);
        memory::realloc_for(context.caller, ptr, context.arg(1) as usize)
            .map(|ptr| ptr.as_ptr() as u64)
            .ok_or(KernelError::AllocationFailed)
    }

    fn syscall_malloc_aligned(&self, context: SyscallContext) -> KernelResult<u64> {
        self.security
            .authorize_memory_service(context.caller)
            .map_err(KernelError::SecurityViolation)?;
        let alignment = context.arg(1) as usize;
        if alignment == 0 || !alignment.is_power_of_two() {
            return Err(KernelError::InvalidArgument);
        }

        memory::malloc_aligned_for(context.caller, context.arg(0) as usize, alignment)
            .map(|ptr| ptr.as_ptr() as u64)
            .ok_or(KernelError::AllocationFailed)
    }

    fn syscall_openat(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let path = path_buf.as_path()?;
        let raw_flags = open_flags_from_libc(context.arg(2) as u32);
        let descriptor_flags = DescriptorFlags::from_open_flags(raw_flags);
        let credentials = self.fs_credentials_for(context.caller)?;
        let file = self
            .root_fs
            .open(path, raw_flags.without_descriptor_flags(), credentials)
            .map_err(KernelError::Filesystem)?;
        let description = self.open_files.insert(file).map_err(map_file_table_error)?;
        match self
            .process_files_mut(context.caller)?
            .open(description, descriptor_flags)
        {
            Ok(fd) => Ok(fd as u64),
            Err(error) => {
                self.close_open_description(description)?;
                Err(map_process_file_table_error(error))
            }
        }
    }

    fn syscall_close(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let descriptor = self
            .process_files_mut(context.caller)?
            .close(context.arg(0) as usize)
            .map_err(map_process_file_table_error)?;
        self.close_open_description(descriptor.description())?;
        Ok(0)
    }

    fn syscall_read(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let buffer = user_slice_mut(context.arg(1), context.arg(2) as usize)?;
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        let file = self
            .open_files
            .get_mut(description)
            .map_err(map_file_table_error)?;
        self.root_fs
            .read(file, buffer)
            .map(|read| read as u64)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_write(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let data = user_slice(context.arg(1), context.arg(2) as usize)?;
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        let file = self
            .open_files
            .get_mut(description)
            .map_err(map_file_table_error)?;
        self.root_fs
            .write(file, data)
            .map(|written| written as u64)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_pread64(&self, context: SyscallContext) -> KernelResult<u64> {
        let buffer = user_slice_mut(context.arg(1), context.arg(2) as usize)?;
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        self.root_fs
            .pread(&file, buffer, context.arg(3))
            .map(|read| read as u64)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_pwrite64(&self, context: SyscallContext) -> KernelResult<u64> {
        let data = user_slice(context.arg(1), context.arg(2) as usize)?;
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        self.root_fs
            .pwrite(&file, data, context.arg(3))
            .map(|written| written as u64)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_lseek(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let fd = context.arg(0) as usize;
        let offset = context.arg(1) as i64;
        let whence = context.arg(2);
        let description = self.fd_description(context.caller, fd)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        let current = file.cursor() as i64;
        let end = self
            .root_fs
            .fstat(&file)
            .map_err(KernelError::Filesystem)?
            .size as i64;
        let base = match whence {
            SEEK_SET => 0,
            SEEK_CUR => current,
            SEEK_END => end,
            _ => return Err(KernelError::InvalidArgument),
        };
        let new_offset = base
            .checked_add(offset)
            .ok_or(KernelError::InvalidArgument)?;
        if new_offset < 0 {
            return Err(KernelError::InvalidArgument);
        }
        self.open_files
            .get_mut(description)
            .map_err(map_file_table_error)?
            .seek(new_offset as u64);
        Ok(new_offset as u64)
    }

    fn syscall_statx(&self, context: SyscallContext) -> KernelResult<u64> {
        self.syscall_newfstatat(context)
    }

    fn syscall_newfstatat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_ptr = context.arg(1);
        let out = user_out_ptr::<CStat>(context.arg(2))?;
        let stat = if path_ptr == 0 {
            let description = self.fd_description(context.caller, context.arg(0) as usize)?;
            let file = self
                .open_files
                .get(description)
                .map_err(map_file_table_error)?;
            self.root_fs.fstat(&file)
        } else {
            let path_buf = self.resolve_user_path(context.caller, context.arg(0), path_ptr)?;
            self.root_fs.stat(path_buf.as_path()?)
        }
        .map_err(KernelError::Filesystem)?;
        unsafe { out.write(CStat::from_kernel(stat)) };
        Ok(0)
    }

    fn syscall_getdents64(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let entries = user_slice_mut_typed::<CDirEntry>(context.arg(1), context.arg(2) as usize)?;
        let fd = context.arg(0) as usize;
        let description = self.fd_description(context.caller, fd)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        let mut cursor = file.cursor() as usize;
        let mut written = 0usize;
        while written < entries.len() {
            let mut kernel_entry = [DirEntry::empty(); 1];
            let count = self
                .root_fs
                .readdir_inode(file.inode(), cursor, &mut kernel_entry)
                .map_err(KernelError::Filesystem)?;
            if count == 0 {
                break;
            }
            cursor += count;
            entries[written] = CDirEntry::from_kernel(&kernel_entry[0], cursor);
            written += 1;
        }
        self.open_files
            .get_mut(description)
            .map_err(map_file_table_error)?
            .advance(written);
        Ok(written as u64)
    }

    fn syscall_mkdirat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let path = path_buf.as_path()?;
        let requested = context.arg(2) as u32;
        let umask = self.process_files(context.caller)?.umask().bits() as u32;
        let credentials = self.fs_credentials_for(context.caller)?;
        let mode = permissions_from_libc_mode(requested & !umask, credentials.uid, credentials.gid);
        self.root_fs
            .mkdir(path, mode, credentials)
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_unlinkat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let path = path_buf.as_path()?;
        let flags = context.arg(2);
        let result = if (flags & AT_REMOVEDIR) != 0 {
            self.root_fs
                .rmdir(path, self.fs_credentials_for(context.caller)?)
        } else {
            self.root_fs
                .unlink(path, self.fs_credentials_for(context.caller)?)
        };
        result.map(|_| 0).map_err(KernelError::Filesystem)
    }

    fn syscall_renameat2(&self, context: SyscallContext) -> KernelResult<u64> {
        let old_path_buf =
            self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let new_path_buf =
            self.resolve_user_path(context.caller, context.arg(2), context.arg(3))?;
        let flags = context.arg(4);
        if flags & !RENAME_NOREPLACE != 0 {
            return Err(KernelError::InvalidArgument);
        }
        if flags & RENAME_NOREPLACE != 0 && self.root_fs.stat(new_path_buf.as_path()?).is_ok() {
            return Err(KernelError::Filesystem(VfsError::AlreadyExists));
        }
        self.root_fs
            .rename(
                old_path_buf.as_path()?,
                new_path_buf.as_path()?,
                self.fs_credentials_for(context.caller)?,
            )
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_ftruncate(&self, context: SyscallContext) -> KernelResult<u64> {
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        self.root_fs
            .ftruncate(
                &file,
                context.arg(1),
                self.fs_credentials_for(context.caller)?,
            )
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_fsync(&self, context: SyscallContext) -> KernelResult<u64> {
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        self.root_fs
            .fsync(&file)
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_mount(&self, context: SyscallContext) -> KernelResult<u64> {
        if context.arg(0) != 0 {
            let _source = user_cstr(context.arg(0))?;
        }
        let target = self.user_path(context.arg(1))?;
        let filesystem_type = if context.arg(2) == 0 {
            DEFAULT_ROOT_FILESYSTEM
        } else {
            user_cstr(context.arg(2))?
        };
        if !target.is_root() || !is_supported_root_filesystem(filesystem_type) {
            return Err(KernelError::Filesystem(VfsError::Unsupported));
        }
        Ok(0)
    }

    fn syscall_chdir(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, AT_FDCWD as u64, context.arg(0))?;
        let stat = self
            .root_fs
            .stat(path_buf.as_path()?)
            .map_err(KernelError::Filesystem)?;
        if stat.kind != crate::kernel::fs::inode::InodeKind::Directory {
            return Err(KernelError::Filesystem(VfsError::NotDirectory));
        }
        self.process_files_mut(context.caller)?
            .set_cwd(ProcessPath::from_path(path_buf.as_path()?));
        Ok(0)
    }

    fn syscall_fchdir(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let description = self.fd_description(context.caller, context.arg(0) as usize)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        let stat = self.root_fs.fstat(&file).map_err(KernelError::Filesystem)?;
        if stat.kind != crate::kernel::fs::inode::InodeKind::Directory {
            return Err(KernelError::Filesystem(VfsError::NotDirectory));
        }
        let path = Path::new(file.path()).map_err(map_path_error)?;
        self.process_files_mut(context.caller)?
            .set_cwd(ProcessPath::from_path(path));
        Ok(0)
    }

    fn syscall_getcwd(&self, context: SyscallContext) -> KernelResult<u64> {
        let buffer = user_slice_mut(context.arg(0), context.arg(1) as usize)?;
        let cwd = self.process_files(context.caller)?.cwd();
        let raw = cwd.as_str().as_bytes();
        if buffer.len() < raw.len() + 1 {
            return Err(KernelError::InvalidArgument);
        }
        buffer[..raw.len()].copy_from_slice(raw);
        buffer[raw.len()] = 0;
        Ok(context.arg(0))
    }

    fn syscall_faccessat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let mode = context.arg(2) as u32;
        if mode
            & !(crate::kernel::fs::stdlib::R_OK
                | crate::kernel::fs::stdlib::W_OK
                | crate::kernel::fs::stdlib::X_OK)
            != 0
        {
            return Err(KernelError::InvalidArgument);
        }
        let stat = self
            .root_fs
            .stat(path_buf.as_path()?)
            .map_err(KernelError::Filesystem)?;
        if mode == crate::kernel::fs::stdlib::F_OK {
            return Ok(0);
        }
        let permissions = crate::kernel::fs::Permissions::new(stat.mode, stat.uid, stat.gid);
        let credentials = self.fs_credentials_for(context.caller)?;
        let allowed = match (
            mode & crate::kernel::fs::stdlib::R_OK != 0,
            mode & crate::kernel::fs::stdlib::W_OK != 0,
            mode & crate::kernel::fs::stdlib::X_OK != 0,
        ) {
            (true, true, true) => {
                permissions.allows(credentials, AccessMode::ReadWrite)
                    && permissions.allows(credentials, AccessMode::Execute)
            }
            (true, true, false) => permissions.allows(credentials, AccessMode::ReadWrite),
            (true, false, false) => permissions.allows(credentials, AccessMode::Read),
            (false, true, false) => permissions.allows(credentials, AccessMode::Write),
            (false, false, true) => permissions.allows(credentials, AccessMode::Execute),
            (true, false, true) => {
                permissions.allows(credentials, AccessMode::Read)
                    && permissions.allows(credentials, AccessMode::Execute)
            }
            (false, true, true) => {
                permissions.allows(credentials, AccessMode::Write)
                    && permissions.allows(credentials, AccessMode::Execute)
            }
            _ => true,
        };
        if allowed {
            Ok(0)
        } else {
            Err(KernelError::Filesystem(VfsError::PermissionDenied))
        }
    }

    fn syscall_fchmodat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        self.root_fs
            .chmod(
                path_buf.as_path()?,
                context.arg(2) as u16,
                self.fs_credentials_for(context.caller)?,
            )
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_fchownat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        self.root_fs
            .chown(
                path_buf.as_path()?,
                context.arg(2) as u16,
                context.arg(3) as u16,
                self.fs_credentials_for(context.caller)?,
            )
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_symlinkat(&self, context: SyscallContext) -> KernelResult<u64> {
        let target = self.user_path_str(context.arg(0))?;
        Path::new_unchecked_rooted(target).map_err(map_path_error)?;
        let link_path_buf =
            self.resolve_user_path(context.caller, context.arg(1), context.arg(2))?;
        self.root_fs
            .symlink(
                target,
                link_path_buf.as_path()?,
                self.fs_credentials_for(context.caller)?,
            )
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_readlinkat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path_buf = self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let buffer = user_slice_mut(context.arg(2), context.arg(3) as usize)?;
        self.root_fs
            .readlink(path_buf.as_path()?, buffer)
            .map(|n| n as u64)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_linkat(&self, context: SyscallContext) -> KernelResult<u64> {
        let old_path_buf =
            self.resolve_user_path(context.caller, context.arg(0), context.arg(1))?;
        let new_path_buf =
            self.resolve_user_path(context.caller, context.arg(2), context.arg(3))?;
        let flags = context.arg(4);
        if flags & !AT_SYMLINK_FOLLOW != 0 {
            return Err(KernelError::InvalidArgument);
        }
        self.root_fs
            .link(
                old_path_buf.as_path()?,
                new_path_buf.as_path()?,
                self.fs_credentials_for(context.caller)?,
            )
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn resolve_user_path(
        &self,
        pid: ProcessId,
        dirfd: u64,
        path_ptr: u64,
    ) -> KernelResult<KernelPathBuf> {
        let raw = self.user_path_str(path_ptr)?;
        self.resolve_path_str(pid, dirfd, raw)
    }

    fn resolve_path_str(
        &self,
        pid: ProcessId,
        dirfd: u64,
        raw: &str,
    ) -> KernelResult<KernelPathBuf> {
        Path::new_unchecked_rooted(raw).map_err(map_path_error)?;
        let files = self.process_files(pid)?;
        let root = files.root();
        let root_str = root.as_str();
        let root_len = root.len();
        let mut resolved = if raw.starts_with('/') {
            KernelPathBuf::from_str(root_str)?
        } else if dirfd as i32 == AT_FDCWD {
            KernelPathBuf::from_str(files.cwd().as_str())?
        } else {
            let description = files
                .get(dirfd as usize)
                .map_err(map_process_file_table_error)?
                .description();
            let file = self
                .open_files
                .get(description)
                .map_err(map_file_table_error)?;
            let metadata = self.root_fs.fstat(&file).map_err(KernelError::Filesystem)?;
            if metadata.kind != crate::kernel::fs::inode::InodeKind::Directory {
                return Err(KernelError::Filesystem(VfsError::NotDirectory));
            }
            KernelPathBuf::from_str(file.path())?
        };
        for component in raw.split('/') {
            let before = resolved.len;
            resolved.push_component(component)?;
            if component == ".." && resolved.len < root_len {
                resolved.truncate_to_root(root_len);
            } else if before < root_len {
                resolved.truncate_to_root(root_len);
            }
        }
        resolved.as_path()?;
        Ok(resolved)
    }

    fn exec_image_metadata(
        &self,
        resolved: &KernelPathBuf,
        stat: crate::kernel::fs::inode::Stat,
        entry_point: u64,
        stack_pointer: u64,
    ) -> ExecImageMetadata {
        let (service_daemon, signature) = signed_exec_manifest_for_path(resolved.as_str());
        ExecImageMetadata::new(
            stat.inode.raw(),
            stat.size,
            stat.mode,
            entry_point,
            stack_pointer,
            service_daemon,
            signature,
        )
    }

    fn user_path(&self, path_ptr: u64) -> KernelResult<Path<'_>> {
        let raw = self.user_path_str(path_ptr)?;
        Path::new(raw).map_err(map_path_error)
    }

    fn user_path_str(&self, path_ptr: u64) -> KernelResult<&'_ str> {
        let bytes = user_cstr(path_ptr)?;
        core::str::from_utf8(bytes).map_err(|_| KernelError::InvalidArgument)
    }

    fn replace_process_image(
        &mut self,
        pid: ProcessId,
        current_thread: Option<ThreadId>,
        entry_point: u64,
        stack_pointer: u64,
    ) -> KernelResult<()> {
        let index = self.locate_process(pid)?;
        if let Some(pcb) = self.process_table[index].as_mut() {
            pcb.set_exec_image(entry_point, 0);
            pcb.thread_count = 0;
        }

        self.scheduler.remove_process(pid);
        let mut kept_thread = current_thread;
        if kept_thread.is_none() {
            let mut idx = 0usize;
            while idx < Self::THREAD_CAPACITY {
                if let Some(thread) = self.thread_table[idx] {
                    if thread.process == pid {
                        kept_thread = Some(thread.id);
                        break;
                    }
                }
                idx += 1;
            }
        }

        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(thread) = self.thread_table[idx] {
                if thread.process == pid && Some(thread.id) != kept_thread {
                    self.thread_table[idx] = None;
                }
            }
            idx += 1;
        }

        let thread_id = if let Some(thread_id) = kept_thread {
            let thread_index = self.locate_thread(thread_id)?;
            let priority = self.process_table[index]
                .as_ref()
                .ok_or(KernelError::UnknownProcess)?
                .priority;
            if let Some(tcb) = self.thread_table[thread_index].as_mut() {
                tcb.replace_exec_image(entry_point, stack_pointer);
                tcb.priority = priority;
            }
            thread_id
        } else {
            self.create_thread(
                pid,
                entry_point,
                self.process_table[index]
                    .as_ref()
                    .ok_or(KernelError::UnknownProcess)?
                    .priority,
            )?
        };

        if let Some(pcb) = self.process_table[index].as_mut() {
            pcb.thread_count = 1;
            pcb.state = ProcessState::Ready;
        }
        let priority = self.process_table[index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .priority;
        self.scheduler
            .enqueue(ScheduledThread::new(thread_id, pid, priority))
            .map_err(|_| KernelError::SchedulerFull)
    }

    fn wait_for_child(
        &mut self,
        parent: ProcessId,
        selector: i64,
        status_out: *mut i32,
        options: u64,
    ) -> KernelResult<u64> {
        const WNOHANG: u64 = 1;
        let mut saw_child = false;
        let parent_pgid = self.process_table[self.locate_process(parent)?]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .process_group;
        let mut idx = 0usize;
        while idx < MAX_PROC {
            if let Some(pcb) = self.process_table[idx] {
                if pcb.parent == Some(parent)
                    && wait_selector_matches(selector, pcb.pid, pcb.process_group, parent_pgid)
                {
                    saw_child = true;
                    if pcb.state == ProcessState::Zombie {
                        let status = pcb.exit_status.unwrap_or(ExitStatus::exited(0));
                        if !status_out.is_null() {
                            unsafe { status_out.write(status.raw()) };
                        }
                        let pid = pcb.pid;
                        self.reap_process_at(idx);
                        return Ok(pid.raw());
                    }
                }
            }
            idx += 1;
        }
        if saw_child && (options & WNOHANG) != 0 {
            Ok(0)
        } else if saw_child {
            Err(KernelError::MessageQueueEmpty)
        } else {
            Err(KernelError::UnknownProcess)
        }
    }

    fn reap_process_at(&mut self, index: usize) {
        if let Some(pcb) = self.process_table[index] {
            self.security.revoke_task(pcb.pid);
            self.process_table[index] = None;
            self.timers.release_process(pcb.pid);
        }
    }

    fn queue_signal_to_parent(&mut self, child: ProcessId, signal: u8) -> KernelResult<()> {
        let child_index = self.locate_process(child)?;
        if let Some(parent) = self.process_table[child_index]
            .as_ref()
            .and_then(|pcb| pcb.parent)
        {
            self.queue_signal(parent, signal)?;
        }
        Ok(())
    }

    fn queue_signal(&mut self, pid: ProcessId, signal: u8) -> KernelResult<()> {
        let index = self.locate_process(pid)?;
        self.process_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?
            .queue_signal(signal)
            .map_err(|_| KernelError::InvalidArgument)
    }

    fn send_signal(&mut self, selector: i64, signal: u8) -> KernelResult<u64> {
        if signal == 0 {
            self.ensure_process_exists(ProcessId::new(selector as u64))?;
            return Ok(0);
        }
        let mut delivered = 0u64;
        if selector > 0 {
            self.queue_signal(ProcessId::new(selector as u64), signal)?;
            return Ok(0);
        }
        let target_group = if selector == 0 {
            None
        } else {
            Some(ProcessGroupId::new((-selector) as u64))
        };
        let mut idx = 0usize;
        while idx < MAX_PROC {
            if let Some(pcb) = self.process_table[idx] {
                let matches = target_group
                    .map(|pgid| pcb.process_group == pgid)
                    .unwrap_or(true);
                if matches && pcb.state != ProcessState::Zombie {
                    if self.queue_signal(pcb.pid, signal).is_ok() {
                        delivered += 1;
                    }
                }
            }
            idx += 1;
        }
        if delivered == 0 {
            Err(KernelError::UnknownProcess)
        } else {
            Ok(0)
        }
    }

    fn deliver_signal_checkpoint(&mut self, pid: ProcessId, thread: ThreadId) -> KernelResult<()> {
        let thread_index = self.locate_thread(thread)?;
        let mask = self.thread_table[thread_index]
            .as_ref()
            .ok_or(KernelError::UnknownThread)?
            .signal_mask;
        let process_index = self.locate_process(pid)?;
        let Some(signal) = self.process_table[process_index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?
            .take_deliverable_signal(mask)
        else {
            return Ok(());
        };
        let action = self.process_table[process_index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .signal_actions[signal as usize];
        if action.handler == 0 && matches!(signal, SIGKILL | SIGTERM) {
            self.exit_process(pid, ExitStatus::signaled(signal));
            return Ok(());
        }
        if let Some(tcb) = self.thread_table[thread_index].as_mut() {
            tcb.deliver_signal(signal, action.handler);
        }
        Ok(())
    }

    fn process_credentials(
        &self,
        pid: ProcessId,
    ) -> KernelResult<&crate::kernel::process::ProcessCredentials> {
        let index = self.locate_process(pid)?;
        Ok(&self.process_table[index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .credentials)
    }

    fn process_credentials_mut(
        &mut self,
        pid: ProcessId,
    ) -> KernelResult<&mut crate::kernel::process::ProcessCredentials> {
        let index = self.locate_process(pid)?;
        Ok(&mut self.process_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?
            .credentials)
    }

    fn fs_credentials_for(&self, pid: ProcessId) -> KernelResult<FsCredentials> {
        let credentials = self.process_credentials(pid)?;
        Ok(FsCredentials::user_with_groups(
            credentials.euid,
            credentials.egid,
            credentials.supplementary_groups(),
            credentials.supplementary_group_count(),
        ))
    }

    fn process_files(
        &self,
        pid: ProcessId,
    ) -> KernelResult<&crate::kernel::process::ProcessFileTable<MAX_OPEN_FILES>> {
        let index = self.locate_process(pid)?;
        Ok(&self.process_table[index]
            .as_ref()
            .ok_or(KernelError::UnknownProcess)?
            .files)
    }

    fn process_files_mut(
        &mut self,
        pid: ProcessId,
    ) -> KernelResult<&mut crate::kernel::process::ProcessFileTable<MAX_OPEN_FILES>> {
        let index = self.locate_process(pid)?;
        Ok(&mut self.process_table[index]
            .as_mut()
            .ok_or(KernelError::UnknownProcess)?
            .files)
    }

    fn duplicate_fd(
        &mut self,
        pid: ProcessId,
        old_fd: usize,
        min_fd: usize,
        flags: DescriptorFlags,
    ) -> KernelResult<u64> {
        let description = self.fd_description(pid, old_fd)?;
        self.open_files
            .increment_ref_count(description)
            .map_err(map_file_table_error)?;
        match self
            .process_files_mut(pid)?
            .open_at_or_above(description, flags, min_fd)
        {
            Ok(fd) => Ok(fd as u64),
            Err(error) => {
                let _ = self.close_open_description(description);
                Err(map_process_file_table_error(error))
            }
        }
    }

    fn duplicate_fd_to(
        &mut self,
        pid: ProcessId,
        old_fd: usize,
        new_fd: usize,
        flags: DescriptorFlags,
        reject_same: bool,
    ) -> KernelResult<u64> {
        let description = self.fd_description(pid, old_fd)?;
        if old_fd == new_fd {
            if reject_same {
                return Err(KernelError::InvalidArgument);
            }
            return Ok(new_fd as u64);
        }
        self.open_files
            .increment_ref_count(description)
            .map_err(map_file_table_error)?;
        let replaced = match self
            .process_files_mut(pid)?
            .duplicate_to(new_fd, description, flags)
        {
            Ok(previous) => previous,
            Err(error) => {
                let _ = self.close_open_description(description);
                return Err(map_process_file_table_error(error));
            }
        };
        if let Some(previous) = replaced {
            self.close_open_description(previous.description())?;
        }
        Ok(new_fd as u64)
    }

    fn fd_description(&self, pid: ProcessId, fd: usize) -> KernelResult<FileDescriptionId> {
        self.process_files(pid)?
            .get(fd)
            .map(|descriptor| descriptor.description())
            .map_err(map_process_file_table_error)
    }

    fn inherit_process_file_table(
        &mut self,
        parent: ProcessId,
    ) -> KernelResult<crate::kernel::process::ProcessFileTable<MAX_OPEN_FILES>> {
        let inherited = *self.process_files(parent)?;
        let mut retained = 0usize;
        let descriptors = inherited.descriptors();
        while retained < MAX_OPEN_FILES {
            if let Some(descriptor) = descriptors[retained] {
                if let Err(error) = self
                    .open_files
                    .increment_ref_count(descriptor.description())
                {
                    self.release_inherited_prefix(&inherited, retained);
                    return Err(map_file_table_error(error));
                }
            }
            retained += 1;
        }
        Ok(inherited)
    }

    fn release_inherited_prefix(
        &mut self,
        table: &crate::kernel::process::ProcessFileTable<MAX_OPEN_FILES>,
        count: usize,
    ) {
        let mut idx = 0usize;
        while idx < count {
            if let Some(descriptor) = table.descriptors()[idx] {
                let _ = self.close_open_description(descriptor.description());
            }
            idx += 1;
        }
    }

    fn release_process_file_table(
        &mut self,
        table: &mut crate::kernel::process::ProcessFileTable<MAX_OPEN_FILES>,
    ) {
        let closed = table.clear();
        self.release_description_ids(&closed);
    }

    fn release_description_ids(
        &mut self,
        descriptions: &[Option<FileDescriptionId>; MAX_OPEN_FILES],
    ) {
        for description in descriptions.iter().flatten() {
            let _ = self.close_open_description(*description);
        }
    }

    fn close_open_description(&mut self, description: FileDescriptionId) -> KernelResult<()> {
        if let Some(file) = self
            .open_files
            .close(description)
            .map_err(map_file_table_error)?
        {
            self.root_fs.close(file).map_err(KernelError::Filesystem)?;
        }
        Ok(())
    }

    pub fn tick(&mut self) {
        device::system_timer().tick();
        let timestamp = KERNEL_TIME.tick();
        self.wake_expired_timeouts(timestamp.as_nanos());
        let mut core_index = 0usize;
        while core_index < cpu::MAX_CORES {
            if self.core_states[core_index].online {
                self.run_core(core_index);
            }
            core_index += 1;
        }
    }

    fn wake_expired_timeouts(&mut self, now_ns: u128) {
        while let Some(expired) = self.timers.expire_sleep(now_ns) {
            let _ = self.wake_process_for_timeout(expired.process);
        }

        while let Some(expired) = self.timers.expire_timer(now_ns) {
            let _ = self.wake_process_for_timeout(expired.owner);
        }
    }

    fn wake_process_for_timeout(&mut self, pid: ProcessId) -> KernelResult<()> {
        let index = self.locate_process(pid)?;
        let mut wake_threads = false;
        if let Some(pcb) = self.process_table[index].as_mut() {
            if pcb.state == ProcessState::Blocked {
                pcb.state = ProcessState::Ready;
                wake_threads = true;
            }
        }
        if wake_threads {
            self.make_threads_ready(pid)?;
        }
        Ok(())
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

            if let Err(reason) = self.security.enforce_isolation(scheduled.process) {
                self.handle_isolation_fault(scheduled.process, reason);
                return;
            }

            let _ = self.deliver_signal_checkpoint(scheduled.process, scheduled.thread);
            if self.locate_thread(scheduled.thread).is_err() {
                self.core_states[core_index].idle_cycle();
                return;
            }

            self.core_states[core_index].start_thread(scheduled.thread);

            let mut terminated = false;
            let mut run_outcome = ThreadRunOutcome::TimeSliceComplete;
            if let Some(entry) = self.thread_table.get_mut(thread_index) {
                if let Some(thread) = entry.as_mut() {
                    if thread.state == ThreadState::Terminated {
                        *entry = None;
                        terminated = true;
                    } else {
                        thread.mark_running();
                        run_outcome = x86_64::run_thread_slice(thread);
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

            match run_outcome {
                ThreadRunOutcome::Syscall(trap) => {
                    let context =
                        SyscallContext::new(scheduled.process, Some(trap.thread), trap.args);
                    let result = self
                        .handle_syscall(trap.number, context)
                        .unwrap_or_else(encode_syscall_error);
                    self.write_thread_syscall_result(trap.thread, result);
                    let _ = self.deliver_signal_checkpoint(scheduled.process, trap.thread);
                }
                ThreadRunOutcome::TimerPreempted | ThreadRunOutcome::TimeSliceComplete => {}
            }

            let mut requeue_thread = false;
            if let Some(entry) = self.thread_table.get_mut(thread_index) {
                if let Some(thread) = entry.as_mut() {
                    if thread.state == ThreadState::Running {
                        thread.mark_ready();
                    }
                    requeue_thread = thread.state == ThreadState::Ready;
                }
            }

            if let Some(pcb) = self.process_table[process_index].as_mut() {
                if pcb.state == ProcessState::Running {
                    pcb.state = ProcessState::Ready;
                }
            }

            self.core_states[core_index].finish_cycle();

            if requeue_thread {
                if scheduled.consume_time_slice() {
                    scheduled.reset_time_slice();
                }

                let _ = self.scheduler.requeue(scheduled);
            }
        } else {
            self.core_states[core_index].idle_cycle();
        }
    }

    fn block_process_at_index(&mut self, pid: ProcessId, index: usize) {
        if let Some(pcb) = self.process_table[index].as_mut() {
            pcb.state = ProcessState::Blocked;
        }
        self.scheduler.remove_process(pid);
        self.block_threads_for_process(pid);
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

    fn make_threads_ready(&mut self, pid: ProcessId) -> KernelResult<()> {
        let mut idx = 0usize;
        while idx < Self::THREAD_CAPACITY {
            if let Some(entry) = self.thread_table.get_mut(idx) {
                if let Some(thread) = entry.as_mut() {
                    if thread.process == pid && thread.state == ThreadState::Blocked {
                        thread.mark_ready();
                        if self
                            .scheduler
                            .enqueue(ScheduledThread::new(
                                thread.id,
                                thread.process,
                                thread.priority,
                            ))
                            .is_err()
                        {
                            thread.block();
                            self.rollback_ready_threads(pid, idx);
                            return Err(KernelError::SchedulerFull);
                        }
                    }
                }
            }
            idx += 1;
        }
        Ok(())
    }

    fn rollback_ready_threads(&mut self, pid: ProcessId, before_index: usize) {
        let mut idx = 0usize;
        while idx < before_index {
            if let Some(entry) = self.thread_table.get_mut(idx) {
                if let Some(thread) = entry.as_mut() {
                    if thread.process == pid && thread.state == ThreadState::Ready {
                        thread.block();
                        self.scheduler.remove_thread(thread.id);
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
        let stack_pointer = self.allocate_stack_pointer(slot, id);
        let tcb = ThreadControlBlock::new(id, pid, entry_point, priority, stack_pointer);
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

    fn write_thread_syscall_result(&mut self, thread: ThreadId, result: u64) {
        if let Ok(index) = self.locate_thread(thread) {
            if let Some(tcb) = self.thread_table[index].as_mut() {
                tcb.write_syscall_result(result);
            }
        }
    }

    fn allocate_stack_pointer(&self, slot: usize, thread: ThreadId) -> u64 {
        const USER_STACK_BASE: u64 = 0x0000_7000_0000_0000;
        const USER_STACK_SIZE: u64 = 0x20_000;
        let stack_slot = (slot as u64).saturating_add(thread.raw());
        USER_STACK_BASE.saturating_add(stack_slot.saturating_mul(USER_STACK_SIZE))
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

    fn handle_isolation_fault(&mut self, pid: ProcessId, _reason: IsolationError) {
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

    pub fn enumerate_devices(&self, out: &mut [DeviceDescriptor]) -> usize {
        self.devices.enumerate(out)
    }

    pub fn device_info(&self, id: DeviceId) -> Option<DeviceDescriptor> {
        self.devices.descriptor(id)
    }

    pub fn device_read(
        &self,
        pid: ProcessId,
        id: DeviceId,
        buffer: &mut [u8],
    ) -> KernelResult<usize> {
        let descriptor = self
            .devices
            .descriptor(id)
            .ok_or(KernelError::DeviceNotFound)?;

        self.security
            .authorize_device_access(pid, descriptor.security)
            .map_err(KernelError::SecurityViolation)?;
        if !self.service_registry.claimed_by(pid, id) {
            return Err(KernelError::SecurityViolation(
                IsolationError::PolicyViolation,
            ));
        }

        self.devices
            .read(id, buffer)
            .map_err(KernelError::DeviceFault)
    }

    pub fn device_write(&self, pid: ProcessId, id: DeviceId, data: &[u8]) -> KernelResult<usize> {
        let descriptor = self
            .devices
            .descriptor(id)
            .ok_or(KernelError::DeviceNotFound)?;

        self.security
            .authorize_device_access(pid, descriptor.security)
            .map_err(KernelError::SecurityViolation)?;
        if !self.service_registry.claimed_by(pid, id) {
            return Err(KernelError::SecurityViolation(
                IsolationError::PolicyViolation,
            ));
        }

        self.devices
            .write(id, data)
            .map_err(KernelError::DeviceFault)
    }
}

fn exec_vector_metadata(ptr: u64, max_entries: usize) -> KernelResult<ExecVectorMetadata> {
    if ptr == 0 {
        return Ok(ExecVectorMetadata::empty());
    }

    let entries = user_slice_typed::<u64>(ptr, max_entries + 1)?;
    let mut count = 0usize;
    while count < entries.len() {
        if entries[count] == 0 {
            return Ok(ExecVectorMetadata::new(ptr, count, false));
        }
        count += 1;
    }

    Ok(ExecVectorMetadata::new(ptr, max_entries, true))
}

fn signed_exec_manifest_for_path(
    path: &str,
) -> (Option<ExecServiceDaemon>, Option<ExecSignatureMetadata>) {
    match path {
        "/bin/displayd" | "/sbin/displayd" | "/displayd" => (
            Some(ExecServiceDaemon::Display),
            Some(ExecSignatureMetadata::new(
                "mirage-service-root",
                0x444953504c415944,
            )),
        ),
        "/bin/networkd" | "/sbin/networkd" | "/networkd" => (
            Some(ExecServiceDaemon::Network),
            Some(ExecSignatureMetadata::new(
                "mirage-service-root",
                0x4e4554574f524b44,
            )),
        ),
        "/bin/inputd" | "/sbin/inputd" | "/inputd" => (
            Some(ExecServiceDaemon::Input),
            Some(ExecSignatureMetadata::new(
                "mirage-service-root",
                0x494e50555444414d,
            )),
        ),
        "/bin/l2-driverd" | "/sbin/l2-driverd" | "/l2-driverd" => (
            Some(ExecServiceDaemon::L2Driver),
            Some(ExecSignatureMetadata::new(
                "mirage-driver-root",
                0x4c32445249564552,
            )),
        ),
        _ => (None, None),
    }
}

fn map_file_table_error(error: FileTableError) -> KernelError {
    match error {
        FileTableError::Full => KernelError::FileTableFull,
        FileTableError::InvalidDescriptor => KernelError::Filesystem(VfsError::InvalidHandle),
    }
}

fn map_process_file_table_error(error: ProcessFileTableError) -> KernelError {
    match error {
        ProcessFileTableError::Full => KernelError::FileTableFull,
        ProcessFileTableError::InvalidDescriptor => {
            KernelError::Filesystem(VfsError::InvalidHandle)
        }
    }
}

fn map_path_error(error: PathError) -> KernelError {
    KernelError::Filesystem(VfsError::InvalidPath(error))
}

fn map_service_registry_error(error: ServiceRegistryError) -> KernelError {
    match error {
        ServiceRegistryError::Full => KernelError::ProcessTableFull,
        ServiceRegistryError::AlreadyRegistered
        | ServiceRegistryError::DeviceAlreadyClaimed
        | ServiceRegistryError::DeviceClassMismatch
        | ServiceRegistryError::NotOwner => KernelError::InvalidArgument,
        ServiceRegistryError::NotRegistered => KernelError::UnknownProcess,
        ServiceRegistryError::DeviceNotClaimed => KernelError::DeviceNotFound,
    }
}

fn decode_registry_service_id(raw: u64) -> KernelResult<RegistryServiceId> {
    RegistryServiceId::from_raw(raw).ok_or(KernelError::InvalidArgument)
}

fn startup_service_to_registry(
    service: crate::kernel::spawn::ServiceId,
) -> Option<RegistryServiceId> {
    match service {
        crate::kernel::spawn::ServiceId::Displayd => Some(RegistryServiceId::Displayd),
        crate::kernel::spawn::ServiceId::Networkd => Some(RegistryServiceId::Networkd),
        crate::kernel::spawn::ServiceId::Inputd => Some(RegistryServiceId::Inputd),
        crate::kernel::spawn::ServiceId::L2Subkernel => None,
    }
}

fn validate_user_range(ptr: u64, len: usize) -> KernelResult<()> {
    if len == 0 {
        return Ok(());
    }
    if ptr == 0 {
        return Err(KernelError::InvalidPointer);
    }
    ptr.checked_add(len as u64)
        .filter(|end| *end >= ptr)
        .map(|_| ())
        .ok_or(KernelError::InvalidPointer)
}

fn user_slice(ptr: u64, len: usize) -> KernelResult<&'static [u8]> {
    validate_user_range(ptr, len)?;
    if len == 0 {
        Ok(&[])
    } else {
        Ok(unsafe { core::slice::from_raw_parts(ptr as *const u8, len) })
    }
}

fn user_slice_mut(ptr: u64, len: usize) -> KernelResult<&'static mut [u8]> {
    validate_user_range(ptr, len)?;
    if len == 0 {
        Ok(&mut [])
    } else {
        Ok(unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len) })
    }
}

fn user_slice_typed<T>(ptr: u64, count: usize) -> KernelResult<&'static [T]> {
    let byte_len = count
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(KernelError::InvalidPointer)?;
    validate_user_range(ptr, byte_len)?;
    if count == 0 {
        Ok(&[])
    } else if !(ptr as *const T).is_aligned() {
        Err(KernelError::InvalidPointer)
    } else {
        Ok(unsafe { core::slice::from_raw_parts(ptr as *const T, count) })
    }
}

fn user_slice_mut_typed<T>(ptr: u64, count: usize) -> KernelResult<&'static mut [T]> {
    let byte_len = count
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(KernelError::InvalidPointer)?;
    validate_user_range(ptr, byte_len)?;
    if count == 0 {
        Ok(&mut [])
    } else if !(ptr as *mut T).is_aligned() {
        Err(KernelError::InvalidPointer)
    } else {
        Ok(unsafe { core::slice::from_raw_parts_mut(ptr as *mut T, count) })
    }
}

fn user_out_ptr<T>(ptr: u64) -> KernelResult<*mut T> {
    validate_user_range(ptr, core::mem::size_of::<T>())?;
    if !(ptr as *mut T).is_aligned() {
        return Err(KernelError::InvalidPointer);
    }
    Ok(ptr as *mut T)
}

fn read_user_value<T: Copy>(ptr: u64) -> KernelResult<T> {
    validate_user_range(ptr, core::mem::size_of::<T>())?;
    if !(ptr as *const T).is_aligned() {
        return Err(KernelError::InvalidPointer);
    }
    Ok(unsafe { (ptr as *const T).read() })
}

fn write_user_value<T: Copy>(ptr: u64, value: T) -> KernelResult<()> {
    let out = user_out_ptr::<T>(ptr)?;
    unsafe { out.write(value) };
    Ok(())
}

fn timespec_to_nanos(timespec: MirageTimespec) -> KernelResult<u128> {
    if timespec.tv_sec < 0 || timespec.tv_nsec < 0 || timespec.tv_nsec >= 1_000_000_000 {
        return Err(KernelError::InvalidArgument);
    }
    let seconds = timespec.tv_sec as u128;
    let nanos = timespec.tv_nsec as u128;
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_add(nanos))
        .ok_or(KernelError::InvalidArgument)
}

fn nanos_to_timespec(nanos: u128) -> MirageTimespec {
    MirageTimespec {
        tv_sec: (nanos / 1_000_000_000) as i64,
        tv_nsec: (nanos % 1_000_000_000) as i64,
    }
}

fn timer_to_itimerspec(
    timer: crate::kernel::timer::ProcessTimer,
    now_ns: u128,
) -> MirageItimerspec {
    let remaining = if timer.armed {
        timer.wake_deadline_ns.saturating_sub(now_ns)
    } else {
        0
    };
    MirageItimerspec {
        it_interval: nanos_to_timespec(timer.interval_ns),
        it_value: nanos_to_timespec(remaining),
    }
}

fn map_timer_error(error: TimerError) -> KernelError {
    match error {
        TimerError::Full => KernelError::AllocationFailed,
        TimerError::InvalidTimer => KernelError::InvalidArgument,
    }
}

fn user_cstr(ptr: u64) -> KernelResult<&'static [u8]> {
    validate_user_range(ptr, 1)?;
    let start = ptr as *const u8;
    let mut len = 0usize;
    while len <= MAX_PATH_BYTES {
        let byte = unsafe { start.add(len).read() };
        if byte == 0 {
            return user_slice(ptr, len);
        }
        len += 1;
    }
    Err(KernelError::Filesystem(VfsError::InvalidPath(
        PathError::TooLong,
    )))
}

fn encode_syscall_error(error: KernelError) -> u64 {
    MIRAGE_SYSCALL_ERROR_BIT | syscall_error_code(error).raw()
}

fn syscall_error_code(error: KernelError) -> SyscallErrorCode {
    match error {
        KernelError::ProcessTableFull => SyscallErrorCode::ProcessTableFull,
        KernelError::SchedulerFull => SyscallErrorCode::SchedulerFull,
        KernelError::UnknownProcess => SyscallErrorCode::NoSuchProcess,
        KernelError::UnknownThread => SyscallErrorCode::NoSuchThread,
        KernelError::ThreadTableFull => SyscallErrorCode::ThreadTableFull,
        KernelError::MessageQueueFull => SyscallErrorCode::QueueFull,
        KernelError::MessageQueueEmpty => SyscallErrorCode::QueueEmpty,
        KernelError::SecurityViolation(reason) => isolation_syscall_error_code(reason),
        KernelError::IsolationFault(reason) => isolation_syscall_error_code(reason),
        KernelError::DeviceNotFound => SyscallErrorCode::NoSuchDevice,
        KernelError::DeviceFault(_) => SyscallErrorCode::DeviceFault,
        KernelError::InvalidSyscall => SyscallErrorCode::InvalidSyscall,
        KernelError::InvalidArgument => SyscallErrorCode::InvalidArgument,
        KernelError::InvalidPointer => SyscallErrorCode::BadAddress,
        KernelError::AllocationFailed => SyscallErrorCode::OutOfMemory,
        KernelError::FileTableFull => SyscallErrorCode::OutOfMemory,
        KernelError::Filesystem(error) => vfs_syscall_error_code(error),
    }
}

fn vfs_syscall_error_code(error: VfsError) -> SyscallErrorCode {
    syscall_error_code_from_vfs(error)
}

fn isolation_syscall_error_code(reason: IsolationError) -> SyscallErrorCode {
    match reason {
        IsolationError::UnknownTask => SyscallErrorCode::NoSuchProcess,
        IsolationError::PolicyViolation | IsolationError::CapabilityMissing => {
            SyscallErrorCode::PermissionDenied
        }
    }
}

fn decode_priority(raw: u64) -> KernelResult<ProcessPriority> {
    match raw {
        0 => Ok(ProcessPriority::Critical),
        1 => Ok(ProcessPriority::High),
        2 => Ok(ProcessPriority::Normal),
        3 => Ok(ProcessPriority::Low),
        _ => Err(KernelError::InvalidArgument),
    }
}

fn decode_credentials(raw: u64) -> KernelResult<Credentials> {
    match raw {
        0 => Ok(Credentials::user()),
        1 => Ok(Credentials::system()),
        _ => Err(KernelError::InvalidArgument),
    }
}

fn decode_security_class(raw: u64) -> KernelResult<SecurityClass> {
    match raw {
        0 => Ok(SecurityClass::Public),
        1 => Ok(SecurityClass::Internal),
        2 => Ok(SecurityClass::Confidential),
        3 => Ok(SecurityClass::System),
        _ => Err(KernelError::InvalidArgument),
    }
}

#[cfg(all(test, not(feature = "qfs-std")))]
mod tests {
    use super::*;
    use crate::kernel::memory::{PROT_EXECUTE, PROT_READ, PROT_WRITE};
    use crate::libc;
    use crate::subkernel::{CapabilitySet, IsolationLevel, SecurityLabel};

    fn boot_kernel() -> Kernel<4, 4> {
        let mut kernel = Kernel::<4, 4>::new();
        kernel.bootstrap();
        kernel
    }

    fn process_state(kernel: &Kernel<4, 4>, pid: ProcessId) -> ProcessState {
        let index = kernel.locate_process(pid).unwrap();
        kernel.process_table[index].unwrap().state
    }

    fn first_thread(kernel: &Kernel<4, 4>, pid: ProcessId) -> ThreadId {
        let mut idx = 0usize;
        while idx < Kernel::<4, 4>::THREAD_CAPACITY {
            if let Some(thread) = kernel.thread_table[idx] {
                if thread.process == pid {
                    return thread.id;
                }
            }
            idx += 1;
        }
        panic!("process has no thread")
    }

    fn process_threads_blocked(kernel: &Kernel<4, 4>, pid: ProcessId) -> bool {
        let mut saw_thread = false;
        let mut idx = 0usize;
        while idx < Kernel::<4, 4>::THREAD_CAPACITY {
            if let Some(thread) = kernel.thread_table[idx] {
                if thread.process == pid {
                    saw_thread = true;
                    if thread.state != ThreadState::Blocked {
                        return false;
                    }
                }
            }
            idx += 1;
        }
        saw_thread
    }

    #[test]
    fn bootstrap_services_starts_l2_before_device_daemons() {
        let mut kernel = boot_kernel();

        let report = kernel.bootstrap_services();

        assert!(report.all_running());
        let l2 = report
            .pid(crate::kernel::spawn::ServiceId::L2Subkernel)
            .unwrap();
        assert_eq!(l2.raw(), 1);
        assert_eq!(
            report.state(crate::kernel::spawn::ServiceId::Displayd),
            Some(crate::kernel::spawn::StartupState::Running)
        );
        assert_eq!(
            report.state(crate::kernel::spawn::ServiceId::Networkd),
            Some(crate::kernel::spawn::StartupState::Running)
        );
        assert_eq!(
            report.state(crate::kernel::spawn::ServiceId::Inputd),
            Some(crate::kernel::spawn::StartupState::Running)
        );

        let mut idx = 0usize;
        let mut child_count = 0usize;
        while idx < Kernel::<4, 4>::THREAD_CAPACITY {
            if idx < kernel.process_table.len() {
                if let Some(pcb) = kernel.process_table[idx] {
                    if pcb.pid != l2 {
                        assert_eq!(pcb.parent, Some(l2));
                        child_count += 1;
                    }
                }
            }
            idx += 1;
        }
        assert_eq!(child_count, 3);
    }

    #[test]
    fn receive_or_block_returns_queued_message_without_blocking() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let payload = MessagePayload::from_slice(SecurityClass::Public, b"ping");

        kernel.send_message(pid, pid, payload).unwrap();

        let message = kernel.receive_or_block(pid).unwrap().unwrap();
        assert_eq!(message.sender, pid);
        assert_eq!(message.receiver, pid);
        assert_eq!(&message.payload.data[..message.payload.length], b"ping");
        assert_eq!(process_state(&kernel, pid), ProcessState::Ready);
    }

    #[test]
    fn receive_or_block_atomically_blocks_empty_receiver() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();

        let message = kernel.receive_or_block(pid).unwrap();

        assert!(message.is_none());
        assert_eq!(process_state(&kernel, pid), ProcessState::Blocked);
        assert!(process_threads_blocked(&kernel, pid));
    }

    #[test]
    fn libc_receive_uses_blocking_receive_syscall() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let thread = first_thread(&kernel, pid);
        let mut out = Message::new(
            ProcessId::new(0),
            ProcessId::new(0),
            0,
            MessagePayload::empty(SecurityClass::Public),
        );

        let received =
            libc::receive_ipc_or_block(&mut kernel, pid, Some(thread), &mut out).unwrap();

        assert_eq!(received, None);
        assert_eq!(process_state(&kernel, pid), ProcessState::Blocked);
        assert!(process_threads_blocked(&kernel, pid));
    }

    #[test]
    fn security_errors_preserve_isolation_reason() {
        let mut kernel = boot_kernel();
        let init = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let user = kernel
            .spawn_child_process(init, 0, ProcessPriority::Normal, Credentials::user())
            .unwrap();
        let mut buffer = [0u8; 8];

        assert!(matches!(
            kernel.device_read(user, DeviceId::new(1), &mut buffer),
            Err(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            ))
        ));

        assert!(matches!(
            kernel.send_message(
                user,
                ProcessId::new(999),
                MessagePayload::empty(SecurityClass::Public)
            ),
            Err(KernelError::SecurityViolation(IsolationError::UnknownTask))
        ));
    }

    #[test]
    fn service_registry_routes_ipc_and_gates_raw_device_access() {
        let mut kernel = boot_kernel();
        let l2 = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let displayd = kernel
            .spawn_child_process(
                l2,
                0,
                ProcessPriority::High,
                Credentials::new(
                    SecurityLabel::internal(),
                    CapabilitySet::ipc_io(),
                    IsolationLevel::Process,
                ),
            )
            .unwrap();
        let user = kernel
            .spawn_child_process(l2, 0, ProcessPriority::Normal, Credentials::user())
            .unwrap();

        kernel
            .register_service(l2, RegistryServiceId::Displayd, displayd)
            .unwrap();

        let payload = MessagePayload::from_slice(SecurityClass::Public, b"draw");
        kernel
            .send_service_message(user, RegistryServiceId::Displayd, payload)
            .unwrap();

        let message = kernel.receive_message(displayd).unwrap();
        assert_eq!(message.sender, user);
        assert_eq!(message.receiver, displayd);
        assert_eq!(&message.payload.data[..message.payload.length], b"draw");

        let mut buffer = [0u8; 64];
        assert!(matches!(
            kernel.device_read(displayd, DeviceId::new(5), &mut buffer),
            Err(KernelError::SecurityViolation(
                IsolationError::PolicyViolation
            ))
        ));

        kernel
            .claim_service_device(displayd, RegistryServiceId::Displayd, DeviceId::new(5))
            .unwrap();
        assert!(kernel
            .device_read(displayd, DeviceId::new(5), &mut buffer)
            .is_ok());
    }

    #[test]
    fn service_registry_revokes_registration_and_claims_on_exit() {
        let mut kernel = boot_kernel();
        let l2 = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let networkd = kernel
            .spawn_child_process(
                l2,
                0,
                ProcessPriority::High,
                Credentials::new(
                    SecurityLabel::internal(),
                    CapabilitySet::ipc_io(),
                    IsolationLevel::Process,
                ),
            )
            .unwrap();

        kernel
            .register_service(l2, RegistryServiceId::Networkd, networkd)
            .unwrap();
        kernel
            .claim_service_device(networkd, RegistryServiceId::Networkd, DeviceId::new(6))
            .unwrap();
        assert_eq!(
            kernel.service_owner(RegistryServiceId::Networkd),
            Some(networkd)
        );

        kernel.exit_process(networkd, ExitStatus::exited(0));

        assert_eq!(kernel.service_owner(RegistryServiceId::Networkd), None);
        assert!(matches!(
            kernel.device_write(networkd, DeviceId::new(6), b"ping"),
            Err(KernelError::SecurityViolation(IsolationError::UnknownTask))
        ));
    }

    #[test]
    fn syscall_error_encoding_maps_structured_security_reasons() {
        assert_eq!(
            encode_syscall_error(KernelError::SecurityViolation(
                IsolationError::CapabilityMissing
            )),
            MIRAGE_SYSCALL_ERROR_BIT | SyscallErrorCode::PermissionDenied.raw()
        );
        assert_eq!(
            encode_syscall_error(KernelError::SecurityViolation(IsolationError::UnknownTask)),
            MIRAGE_SYSCALL_ERROR_BIT | SyscallErrorCode::NoSuchProcess.raw()
        );
        assert_eq!(
            encode_syscall_error(KernelError::MessageQueueFull),
            MIRAGE_SYSCALL_ERROR_BIT | SyscallErrorCode::QueueFull.raw()
        );
    }

    #[test]
    fn nanosleep_blocks_until_kernel_time_deadline() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let thread = first_thread(&kernel, pid);
        let req = MirageTimespec {
            tv_sec: 0,
            tv_nsec: 1,
        };

        kernel
            .handle_syscall(
                SyscallNumber::Nanosleep.raw(),
                SyscallContext::new(
                    pid,
                    Some(thread),
                    [&req as *const MirageTimespec as u64, 0, 0, 0, 0, 0],
                ),
            )
            .unwrap();

        assert_eq!(process_state(&kernel, pid), ProcessState::Blocked);
        assert!(process_threads_blocked(&kernel, pid));

        kernel.tick();

        assert_eq!(process_state(&kernel, pid), ProcessState::Ready);
        assert_eq!(
            kernel.thread_table[kernel.locate_thread(thread).unwrap()]
                .unwrap()
                .state,
            ThreadState::Ready
        );
    }

    #[test]
    fn nanosleep_rejects_malformed_timespec() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let req = MirageTimespec {
            tv_sec: 0,
            tv_nsec: 1_000_000_000,
        };

        assert!(matches!(
            kernel.handle_syscall(
                SyscallNumber::Nanosleep.raw(),
                SyscallContext::new(
                    pid,
                    None,
                    [&req as *const MirageTimespec as u64, 0, 0, 0, 0, 0,],
                ),
            ),
            Err(KernelError::InvalidArgument)
        ));
        assert_eq!(process_state(&kernel, pid), ProcessState::Ready);
    }

    #[test]
    fn process_timers_are_owned_and_reject_bad_ids() {
        let mut kernel = boot_kernel();
        let owner = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let other = kernel
            .spawn_child_process(owner, 0, ProcessPriority::Normal, Credentials::user())
            .unwrap();
        let mut timer_id = 0u64;

        kernel
            .handle_syscall(
                SyscallNumber::TimerCreate.raw(),
                SyscallContext::new(
                    owner,
                    None,
                    [0, 0, &mut timer_id as *mut u64 as u64, 0, 0, 0],
                ),
            )
            .unwrap();
        assert_ne!(timer_id, 0);

        assert!(matches!(
            kernel.handle_syscall(
                SyscallNumber::TimerGettime.raw(),
                SyscallContext::new(
                    other,
                    None,
                    [
                        timer_id,
                        &mut MirageItimerspec {
                            it_interval: MirageTimespec {
                                tv_sec: 0,
                                tv_nsec: 0
                            },
                            it_value: MirageTimespec {
                                tv_sec: 0,
                                tv_nsec: 0
                            },
                        } as *mut MirageItimerspec as u64,
                        0,
                        0,
                        0,
                        0
                    ]
                ),
            ),
            Err(KernelError::InvalidArgument)
        ));
    }

    #[test]
    fn timer_settime_arms_gettime_and_delete() {
        let mut kernel = boot_kernel();
        let owner = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let mut timer_id = 0u64;
        let new_value = MirageItimerspec {
            it_interval: MirageTimespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: MirageTimespec {
                tv_sec: 0,
                tv_nsec: 1,
            },
        };
        let mut current = MirageItimerspec {
            it_interval: MirageTimespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: MirageTimespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
        };

        kernel
            .handle_syscall(
                SyscallNumber::TimerCreate.raw(),
                SyscallContext::new(
                    owner,
                    None,
                    [0, 0, &mut timer_id as *mut u64 as u64, 0, 0, 0],
                ),
            )
            .unwrap();
        kernel
            .handle_syscall(
                SyscallNumber::TimerSettime.raw(),
                SyscallContext::new(
                    owner,
                    None,
                    [
                        timer_id,
                        0,
                        &new_value as *const MirageItimerspec as u64,
                        0,
                        0,
                        0,
                    ],
                ),
            )
            .unwrap();
        kernel
            .handle_syscall(
                SyscallNumber::TimerGettime.raw(),
                SyscallContext::new(
                    owner,
                    None,
                    [
                        timer_id,
                        &mut current as *mut MirageItimerspec as u64,
                        0,
                        0,
                        0,
                        0,
                    ],
                ),
            )
            .unwrap();
        assert!(current.it_value.tv_sec > 0 || current.it_value.tv_nsec > 0);

        kernel
            .handle_syscall(
                SyscallNumber::TimerDelete.raw(),
                SyscallContext::new(owner, None, [timer_id, 0, 0, 0, 0, 0]),
            )
            .unwrap();
        assert!(matches!(
            kernel.handle_syscall(
                SyscallNumber::TimerDelete.raw(),
                SyscallContext::new(owner, None, [timer_id, 0, 0, 0, 0, 0]),
            ),
            Err(KernelError::InvalidArgument)
        ));
    }

    #[test]
    fn process_file_tables_share_inherited_open_descriptions() {
        let mut kernel = boot_kernel();
        let parent = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let path = b"/fdshare\0";
        let fd = kernel
            .handle_syscall(
                SyscallNumber::OpenAt.raw(),
                SyscallContext::new(
                    parent,
                    None,
                    [
                        AT_FDCWD as u64,
                        path.as_ptr() as u64,
                        (crate::kernel::fs::O_RDWR
                            | crate::kernel::fs::O_CREAT
                            | crate::kernel::fs::O_CLOEXEC) as u64,
                        0,
                        0,
                        0,
                    ],
                ),
            )
            .unwrap() as usize;
        let description = kernel.fd_description(parent, fd).unwrap();
        assert_eq!(kernel.open_files.ref_count(description).unwrap(), 1);
        assert!(kernel
            .process_files(parent)
            .unwrap()
            .get(fd)
            .unwrap()
            .close_on_exec());

        let child = kernel
            .spawn_child_process(parent, 0, ProcessPriority::Normal, Credentials::user())
            .unwrap();
        assert_eq!(kernel.fd_description(child, fd).unwrap(), description);
        assert_eq!(kernel.open_files.ref_count(description).unwrap(), 2);

        kernel
            .handle_syscall(
                SyscallNumber::Close.raw(),
                SyscallContext::new(parent, None, [fd as u64, 0, 0, 0, 0, 0]),
            )
            .unwrap();
        assert_eq!(kernel.open_files.ref_count(description).unwrap(), 1);

        kernel.terminate_process(child);
        assert!(matches!(
            kernel.open_files.ref_count(description),
            Err(FileTableError::InvalidDescriptor)
        ));
    }

    #[test]
    fn memory_syscalls_are_process_owned() {
        let mut kernel = boot_kernel();
        let owner = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let other = kernel
            .spawn_child_process(owner, 0, ProcessPriority::Normal, Credentials::user())
            .unwrap();
        let ptr = libc::malloc(&mut kernel, owner, None, 64).unwrap();

        assert!(matches!(
            libc::free(&mut kernel, other, None, ptr),
            Err(KernelError::InvalidArgument)
        ));
        assert!(libc::free(&mut kernel, owner, None, ptr).is_ok());
    }

    #[test]
    fn mmap_rejects_writable_executable_mapping() {
        let mut kernel = boot_kernel();
        let pid = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let protection = MemoryProtection::from_bits(PROT_READ | PROT_WRITE | PROT_EXECUTE);

        assert!(matches!(
            libc::mmap(&mut kernel, pid, None, 4096, protection),
            Err(KernelError::SecurityViolation(_))
        ));
    }

    #[test]
    fn mmap_rejects_executable_mapping_for_unprivileged_process() {
        let mut kernel = boot_kernel();
        let init = kernel.spawn_initial_process(Credentials::system()).unwrap();
        let user = kernel
            .spawn_child_process(init, 0, ProcessPriority::Normal, Credentials::user())
            .unwrap();
        let protection = MemoryProtection::from_bits(PROT_READ | PROT_EXECUTE);

        assert!(matches!(
            libc::mmap(&mut kernel, user, None, 4096, protection),
            Err(KernelError::SecurityViolation(_))
        ));
    }
}

fn wait_selector_matches(
    selector: i64,
    child_pid: ProcessId,
    child_pgid: ProcessGroupId,
    parent_pgid: ProcessGroupId,
) -> bool {
    if selector == -1 {
        true
    } else if selector == 0 {
        child_pgid == parent_pgid
    } else if selector > 0 {
        child_pid.raw() == selector as u64
    } else {
        child_pgid.raw() == (-selector) as u64
    }
}
