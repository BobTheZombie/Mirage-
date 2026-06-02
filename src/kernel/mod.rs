//! Core kernel primitives: process lifecycle, scheduling, IPC routing, and
//! multi-core orchestration.

pub mod cpu;
pub mod device;
pub mod fs;
pub mod ipc;
pub mod memory;
pub mod process;
pub mod scheduler;
pub mod sync;
pub mod syscall;
pub mod thread;
pub mod time;

use crate::arch::x86_64::{self, clock, ThreadRunOutcome};
use crate::kernel::cpu::CpuCoreState;
use crate::kernel::device::{
    DeviceDescriptor, DeviceError as DriverError, DeviceId, DeviceKind, DeviceManager,
    MirageDeviceDescriptor,
};
use crate::kernel::fs::{
    DescriptorFlags, DirEntry, FileDescriptionId, FileSystem, FileTable, FileTableError,
    FsCredentials, OpenFlags, Path, PathError, Permissions, SsdUsbFileSystem, Stat, VfsError,
    MAX_PATH_BYTES,
};
use crate::kernel::ipc::{Message, MessagePayload, MessageQueue, MessageQueueError};
use crate::kernel::memory::MemoryProtection;
use crate::kernel::process::{
    ProcessControlBlock, ProcessFileTableError, ProcessId, ProcessPriority, ProcessState,
};
use crate::kernel::scheduler::{ScheduledThread, Scheduler};
use crate::kernel::syscall::{
    SyscallContext, SyscallErrorCode, SyscallNumber, MIRAGE_SYSCALL_ERROR_BIT,
};
use crate::kernel::thread::{CpuContext, ThreadControlBlock, ThreadId, ThreadState, MAX_THREADS};
use crate::kernel::time::KERNEL_TIME;
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
    root_fs: SsdUsbFileSystem,
    open_files: FileTable<MAX_OPEN_FILES>,
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
            devices: DeviceManager::new(),
            root_fs: SsdUsbFileSystem::new_on_block_device(
                false,
                crate::kernel::device::built_in_block_storage(),
            ),
            open_files: FileTable::new(),
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
        self.devices.reset();
        self.open_files.clear();
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

        self.devices.install_core_devices();
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

    pub fn spawn_child_process(
        &mut self,
        parent_pid: ProcessId,
        entry_point: u64,
        priority: ProcessPriority,
        requested_creds: Credentials,
    ) -> KernelResult<ProcessId> {
        self.ensure_process_exists(parent_pid)?;
        self.security
            .authorize_spawn(parent_pid, requested_creds)
            .map_err(KernelError::SecurityViolation)?;

        self.spawn_process(entry_point, priority, Some(parent_pid), requested_creds)
    }

    fn spawn_process(
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
        if let Some(parent_pid) = parent {
            pcb.files = self.inherit_process_file_table(parent_pid)?;
        }

        self.security.register_task(pid, creds).map_err(|err| {
            self.release_process_file_table(&mut pcb.files);
            KernelError::SecurityViolation(err)
        })?;

        self.process_table[slot] = Some(pcb);

        let thread_id = match self.create_thread(pid, entry_point, priority) {
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
            if let Some(mut pcb) = self.process_table[index].take() {
                self.release_process_file_table(&mut pcb.files);
            }
            self.ipc_queues[index].clear();
            self.scheduler.remove_process(pid);
            self.remove_threads_for_process(pid);
            memory::release_process(pid);
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
        }
    }

    fn syscall_spawn(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let entry_point = context.arg(0);
        let priority = decode_priority(context.arg(1))?;
        let credentials = decode_credentials(context.arg(2))?;
        self.spawn_child_process(context.caller, entry_point, priority, credentials)
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
        let path = self.absolute_user_path(context.arg(0), context.arg(1))?;
        let raw_flags = OpenFlags::from_bits(context.arg(2) as u32);
        let descriptor_flags = DescriptorFlags::from_open_flags(raw_flags);
        let credentials = fs_credentials_for(context.caller);
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
        let out = user_out_ptr::<Stat>(context.arg(2))?;
        let stat = if path_ptr == 0 {
            let description = self.fd_description(context.caller, context.arg(0) as usize)?;
            let file = self
                .open_files
                .get(description)
                .map_err(map_file_table_error)?;
            self.root_fs.fstat(&file)
        } else {
            let path = self.absolute_user_path(context.arg(0), path_ptr)?;
            self.root_fs.stat(path)
        }
        .map_err(KernelError::Filesystem)?;
        unsafe { out.write(stat) };
        Ok(0)
    }

    fn syscall_getdents64(&mut self, context: SyscallContext) -> KernelResult<u64> {
        let entries = user_slice_mut_typed::<DirEntry>(context.arg(1), context.arg(2) as usize)?;
        let fd = context.arg(0) as usize;
        let description = self.fd_description(context.caller, fd)?;
        let file = self
            .open_files
            .get(description)
            .map_err(map_file_table_error)?;
        let count = self
            .root_fs
            .readdir_inode(file.inode(), file.cursor() as usize, entries)
            .map_err(KernelError::Filesystem)?;
        self.open_files
            .get_mut(description)
            .map_err(map_file_table_error)?
            .advance(count);
        Ok(count as u64)
    }

    fn syscall_mkdirat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path = self.absolute_user_path(context.arg(0), context.arg(1))?;
        let requested = context.arg(2) as u16;
        let umask = self.process_files(context.caller)?.umask().bits();
        let mode = Permissions::new(requested & !umask, context.caller.raw() as u16, 0);
        self.root_fs
            .mkdir(path, mode, fs_credentials_for(context.caller))
            .map(|_| 0)
            .map_err(KernelError::Filesystem)
    }

    fn syscall_unlinkat(&self, context: SyscallContext) -> KernelResult<u64> {
        let path = self.absolute_user_path(context.arg(0), context.arg(1))?;
        let flags = context.arg(2);
        let result = if (flags & 0x200) != 0 {
            self.root_fs.rmdir(path, fs_credentials_for(context.caller))
        } else {
            self.root_fs
                .unlink(path, fs_credentials_for(context.caller))
        };
        result.map(|_| 0).map_err(KernelError::Filesystem)
    }

    fn syscall_renameat2(&self, context: SyscallContext) -> KernelResult<u64> {
        let old_path = self.absolute_user_path(context.arg(0), context.arg(1))?;
        let new_path = self.absolute_user_path(context.arg(2), context.arg(3))?;
        self.root_fs
            .rename(old_path, new_path, fs_credentials_for(context.caller))
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
            .ftruncate(&file, context.arg(1), fs_credentials_for(context.caller))
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
        if context.arg(2) != 0 {
            let _filesystem_type = user_cstr(context.arg(2))?;
        }
        if !target.is_root() {
            return Err(KernelError::Filesystem(VfsError::Unsupported));
        }
        Ok(0)
    }

    fn absolute_user_path(&self, dirfd: u64, path_ptr: u64) -> KernelResult<Path<'_>> {
        let path = self.user_path(path_ptr)?;
        if !path.as_str().starts_with('/') && dirfd as i32 != AT_FDCWD {
            return Err(KernelError::InvalidArgument);
        }
        Ok(path)
    }

    fn user_path(&self, path_ptr: u64) -> KernelResult<Path<'_>> {
        let bytes = user_cstr(path_ptr)?;
        let raw = core::str::from_utf8(bytes).map_err(|_| KernelError::InvalidArgument)?;
        Path::new(raw).map_err(map_path_error)
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
        for description in table.clear().iter().flatten() {
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

            if let Err(reason) = self.security.enforce_isolation(scheduled.process) {
                self.handle_isolation_fault(scheduled.process, reason);
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

        self.devices
            .write(id, data)
            .map_err(KernelError::DeviceFault)
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

fn fs_credentials_for(pid: ProcessId) -> FsCredentials {
    FsCredentials::user(pid.raw() as u16, 0)
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
    match error {
        VfsError::InvalidPath(PathError::TooLong)
        | VfsError::InvalidPath(PathError::ComponentTooLong)
        | VfsError::NameTooLong => SyscallErrorCode::NameTooLong,
        VfsError::InvalidPath(PathError::Empty) | VfsError::NotFound => {
            SyscallErrorCode::FileNotFound
        }
        VfsError::InvalidPath(PathError::NotAbsolute)
        | VfsError::InvalidPath(PathError::InvalidByte)
        | VfsError::InvalidArgument => SyscallErrorCode::InvalidArgument,
        VfsError::NotDirectory => SyscallErrorCode::NotDirectory,
        VfsError::IsDirectory => SyscallErrorCode::IsDirectory,
        VfsError::AlreadyExists => SyscallErrorCode::AlreadyExists,
        VfsError::PermissionDenied => SyscallErrorCode::PermissionDenied,
        VfsError::ReadOnly => SyscallErrorCode::ReadOnlyFilesystem,
        VfsError::NoSpace => SyscallErrorCode::NoSpace,
        VfsError::InvalidHandle => SyscallErrorCode::BadFileDescriptor,
        VfsError::Busy => SyscallErrorCode::FilesystemBusy,
        VfsError::CrossDevice => SyscallErrorCode::CrossDevice,
        VfsError::TooManyLinks => SyscallErrorCode::TooManyLinks,
        VfsError::Unsupported => SyscallErrorCode::UnsupportedFilesystem,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::memory::{PROT_EXECUTE, PROT_READ, PROT_WRITE};
    use crate::libc;

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
                        OpenFlags::RDWR
                            .union(OpenFlags::CREATE)
                            .union(OpenFlags::CLOSE_ON_EXEC)
                            .bits() as u64,
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
