//! Filesystem and file-descriptor service seam.

use crate::kernel::fs::{CDirEntry, CStat};
use crate::kernel::process::ProcessId;
use crate::kernel::syscall::{SyscallContext, SyscallNumber, SYSCALL_MAX_ARGS};
use crate::kernel::thread::ThreadId;
use crate::kernel::{Kernel, KernelResult};

/// Kernel-internal adapter for descriptor-table and VFS operations.
///
/// The default `Kernel` implementation deliberately enters through
/// `handle_syscall`, keeping ABI validation, path resolution, descriptor table
/// updates, and VFS error mapping in one place.
pub trait FileSystemService {
    fn openat(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        dirfd: i32,
        path: *const u8,
        flags: u32,
        mode: u32,
    ) -> KernelResult<i32>;

    fn close(&mut self, caller: ProcessId, thread: Option<ThreadId>, fd: i32) -> KernelResult<()>;

    fn read(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        fd: i32,
        buffer: &mut [u8],
    ) -> KernelResult<usize>;

    fn write(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        fd: i32,
        data: &[u8],
    ) -> KernelResult<usize>;

    fn lseek(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        fd: i32,
        offset: i64,
        whence: u32,
    ) -> KernelResult<u64>;

    fn fstat(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        fd: i32,
        out: &mut CStat,
    ) -> KernelResult<()>;

    fn statx(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        dirfd: i32,
        path: *const u8,
        flags: u32,
        mask: u32,
        out: &mut CStat,
    ) -> KernelResult<()>;

    fn mkdirat(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        dirfd: i32,
        path: *const u8,
        mode: u32,
    ) -> KernelResult<()>;

    fn unlinkat(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        dirfd: i32,
        path: *const u8,
        flags: u32,
    ) -> KernelResult<()>;

    fn renameat2(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        old_dirfd: i32,
        old_path: *const u8,
        new_dirfd: i32,
        new_path: *const u8,
        flags: u32,
    ) -> KernelResult<()>;

    fn fsync(&mut self, caller: ProcessId, thread: Option<ThreadId>, fd: i32) -> KernelResult<()>;

    fn ftruncate(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        fd: i32,
        size: u64,
    ) -> KernelResult<()>;

    fn getdents64(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        fd: i32,
        out: &mut [CDirEntry],
    ) -> KernelResult<usize>;
}

impl<const MAX_PROC: usize, const MSG_DEPTH: usize> FileSystemService
    for Kernel<MAX_PROC, MSG_DEPTH>
{
    fn openat(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        dirfd: i32,
        path: *const u8,
        flags: u32,
        mode: u32,
    ) -> KernelResult<i32> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::OpenAt,
            [dirfd as u64, path as u64, flags as u64, mode as u64, 0, 0],
        )
        .map(|fd| fd as i32)
    }

    fn close(&mut self, caller: ProcessId, thread: Option<ThreadId>, fd: i32) -> KernelResult<()> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::Close,
            [fd as u64, 0, 0, 0, 0, 0],
        )
        .map(|_| ())
    }

    fn read(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        fd: i32,
        buffer: &mut [u8],
    ) -> KernelResult<usize> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::Read,
            [
                fd as u64,
                buffer.as_mut_ptr() as u64,
                buffer.len() as u64,
                0,
                0,
                0,
            ],
        )
        .map(|read| read as usize)
    }

    fn write(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        fd: i32,
        data: &[u8],
    ) -> KernelResult<usize> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::Write,
            [fd as u64, data.as_ptr() as u64, data.len() as u64, 0, 0, 0],
        )
        .map(|written| written as usize)
    }

    fn lseek(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        fd: i32,
        offset: i64,
        whence: u32,
    ) -> KernelResult<u64> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::Lseek,
            [fd as u64, offset as u64, whence as u64, 0, 0, 0],
        )
    }

    fn fstat(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        fd: i32,
        out: &mut CStat,
    ) -> KernelResult<()> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::NewFstatAt,
            [fd as u64, 0, out as *mut CStat as u64, 0, 0, 0],
        )
        .map(|_| ())
    }

    fn statx(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        dirfd: i32,
        path: *const u8,
        flags: u32,
        mask: u32,
        out: &mut CStat,
    ) -> KernelResult<()> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::Statx,
            [
                dirfd as u64,
                path as u64,
                flags as u64,
                mask as u64,
                out as *mut CStat as u64,
                0,
            ],
        )
        .map(|_| ())
    }

    fn mkdirat(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        dirfd: i32,
        path: *const u8,
        mode: u32,
    ) -> KernelResult<()> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::MkdirAt,
            [dirfd as u64, path as u64, mode as u64, 0, 0, 0],
        )
        .map(|_| ())
    }

    fn unlinkat(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        dirfd: i32,
        path: *const u8,
        flags: u32,
    ) -> KernelResult<()> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::UnlinkAt,
            [dirfd as u64, path as u64, flags as u64, 0, 0, 0],
        )
        .map(|_| ())
    }

    fn renameat2(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        old_dirfd: i32,
        old_path: *const u8,
        new_dirfd: i32,
        new_path: *const u8,
        flags: u32,
    ) -> KernelResult<()> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::RenameAt2,
            [
                old_dirfd as u64,
                old_path as u64,
                new_dirfd as u64,
                new_path as u64,
                flags as u64,
                0,
            ],
        )
        .map(|_| ())
    }

    fn fsync(&mut self, caller: ProcessId, thread: Option<ThreadId>, fd: i32) -> KernelResult<()> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::Fsync,
            [fd as u64, 0, 0, 0, 0, 0],
        )
        .map(|_| ())
    }

    fn ftruncate(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        fd: i32,
        size: u64,
    ) -> KernelResult<()> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::Ftruncate,
            [fd as u64, size, 0, 0, 0, 0],
        )
        .map(|_| ())
    }

    fn getdents64(
        &mut self,
        caller: ProcessId,
        thread: Option<ThreadId>,
        fd: i32,
        out: &mut [CDirEntry],
    ) -> KernelResult<usize> {
        service_syscall(
            self,
            caller,
            thread,
            SyscallNumber::Getdents64,
            [
                fd as u64,
                out.as_mut_ptr() as u64,
                out.len() as u64,
                0,
                0,
                0,
            ],
        )
        .map(|count| count as usize)
    }
}

fn service_syscall<const MAX_PROC: usize, const MSG_DEPTH: usize>(
    kernel: &mut Kernel<MAX_PROC, MSG_DEPTH>,
    caller: ProcessId,
    thread: Option<ThreadId>,
    number: SyscallNumber,
    args: [u64; SYSCALL_MAX_ARGS],
) -> KernelResult<u64> {
    kernel.handle_syscall(number.raw(), SyscallContext::new(caller, thread, args))
}
