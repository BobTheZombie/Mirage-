//! C/POSIX ABI adapters for kernel VFS types.
//!
//! Kernel filesystem code uses compact, Mirage-native structures such as
//! [`OpenFlags`], [`Permissions`], [`Stat`], and [`DirEntry`].  This module is
//! the single translation point for libc/syscall-facing constants and
//! C-compatible payloads so wrappers and syscall glue do not duplicate ABI
//! conversions.

use crate::kernel::{
    fs::{
        file::OpenFlags,
        inode::{DirEntry, InodeKind, Stat},
        path::{PathError, MAX_COMPONENT_BYTES},
        permissions::Permissions,
        vfs::VfsError,
    },
    syscall::SyscallErrorCode,
};

/// `open(2)` access-mode mask.
pub const O_ACCMODE: u32 = 0o00000003;
pub const O_RDONLY: u32 = 0o00000000;
pub const O_WRONLY: u32 = 0o00000001;
pub const O_RDWR: u32 = 0o00000002;

/// `open(2)` creation/status/descriptor flags using Linux-compatible values.
pub const O_CREAT: u32 = 0o00000100;
pub const O_EXCL: u32 = 0o00000200;
pub const O_TRUNC: u32 = 0o00001000;
pub const O_APPEND: u32 = 0o00002000;
pub const O_DIRECTORY: u32 = 0o00200000;
pub const O_NOFOLLOW: u32 = 0o00400000;
pub const O_CLOEXEC: u32 = 0o02000000;

/// `access(2)` mode constants.
pub const F_OK: u32 = 0;
pub const X_OK: u32 = 1;
pub const W_OK: u32 = 2;
pub const R_OK: u32 = 4;

/// Linux-compatible file type bits included in `st_mode` and `d_type` values.
pub const S_IFMT: u16 = 0o170000;
pub const S_IFIFO: u16 = 0o010000;
pub const S_IFCHR: u16 = 0o020000;
pub const S_IFDIR: u16 = 0o040000;
pub const S_IFBLK: u16 = 0o060000;
pub const S_IFREG: u16 = 0o100000;
pub const S_IFLNK: u16 = 0o120000;
pub const S_IFSOCK: u16 = 0o140000;

pub const DT_UNKNOWN: u8 = 0;
pub const DT_FIFO: u8 = 1;
pub const DT_CHR: u8 = 2;
pub const DT_DIR: u8 = 4;
pub const DT_BLK: u8 = 6;
pub const DT_REG: u8 = 8;
pub const DT_LNK: u8 = 10;
pub const DT_SOCK: u8 = 12;

/// Converts libc-style `O_*` bits into Mirage-native VFS open flags.
pub const fn open_flags_from_libc(flags: u32) -> OpenFlags {
    let mut translated = match flags & O_ACCMODE {
        O_WRONLY => OpenFlags::WRONLY,
        O_RDWR => OpenFlags::RDWR,
        _ => OpenFlags::RDONLY,
    };
    if (flags & O_CREAT) != 0 {
        translated = translated.union(OpenFlags::CREATE);
    }
    if (flags & O_EXCL) != 0 {
        translated = translated.union(OpenFlags::EXCLUSIVE);
    }
    if (flags & O_TRUNC) != 0 {
        translated = translated.union(OpenFlags::TRUNCATE);
    }
    if (flags & O_APPEND) != 0 {
        translated = translated.union(OpenFlags::APPEND);
    }
    if (flags & O_DIRECTORY) != 0 {
        translated = translated.union(OpenFlags::DIRECTORY);
    }
    if (flags & O_NOFOLLOW) != 0 {
        translated = translated.union(OpenFlags::NOFOLLOW);
    }
    if (flags & O_CLOEXEC) != 0 {
        translated = translated.union(OpenFlags::CLOSE_ON_EXEC);
    }
    translated
}

/// Converts libc mode bits to Mirage permissions for a newly-created node.
pub const fn permissions_from_libc_mode(mode: u32, owner: u16, group: u16) -> Permissions {
    Permissions::new((mode as u16) & 0o777, owner, group)
}

/// Central VFS errno adapter for libc/syscall ABI callers.
pub const fn errno_from_vfs(error: VfsError) -> i32 {
    error.linux_errno()
}

pub const fn negative_errno_from_vfs(error: VfsError) -> isize {
    -(errno_from_vfs(error) as isize)
}

/// Converts a VFS failure to the structured syscall error code carried by trap returns.
pub const fn syscall_error_code_from_vfs(error: VfsError) -> SyscallErrorCode {
    match error {
        VfsError::InvalidPath(PathError::TooLong)
        | VfsError::InvalidPath(PathError::ComponentTooLong)
        | VfsError::NameTooLong => SyscallErrorCode::NameTooLong,
        VfsError::NoDevice | VfsError::InvalidPath(PathError::Empty) | VfsError::NotFound => {
            SyscallErrorCode::FileNotFound
        }
        VfsError::InvalidSuperblock
        | VfsError::CorruptFilesystem
        | VfsError::InvalidPath(PathError::NotAbsolute)
        | VfsError::InvalidPath(PathError::InvalidByte)
        | VfsError::InvalidArgument
        | VfsError::InvalidInput => SyscallErrorCode::InvalidArgument,
        VfsError::NotDirectory => SyscallErrorCode::NotDirectory,
        VfsError::IsDirectory => SyscallErrorCode::IsDirectory,
        VfsError::AlreadyExists => SyscallErrorCode::AlreadyExists,
        VfsError::PermissionDenied => SyscallErrorCode::PermissionDenied,
        VfsError::ReadOnly | VfsError::JournalRequired => SyscallErrorCode::ReadOnlyFilesystem,
        VfsError::Io | VfsError::ChecksumMismatch => SyscallErrorCode::DeviceFault,
        VfsError::NoSpace => SyscallErrorCode::NoSpace,
        VfsError::InvalidHandle => SyscallErrorCode::BadFileDescriptor,
        VfsError::Busy => SyscallErrorCode::FilesystemBusy,
        VfsError::CrossDevice => SyscallErrorCode::CrossDevice,
        VfsError::TooManyLinks => SyscallErrorCode::TooManyLinks,
        VfsError::Unsupported | VfsError::UnsupportedFeature => {
            SyscallErrorCode::UnsupportedFilesystem
        }
    }
}

/// C-compatible `stat` payload populated from kernel [`Stat`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CStat {
    pub st_dev: u64,
    pub st_ino: u64,
    pub st_mode: u32,
    pub st_nlink: u64,
    pub st_uid: u32,
    pub st_gid: u32,
    pub st_rdev: u64,
    pub st_size: i64,
    pub st_blksize: i64,
    pub st_blocks: i64,
    pub st_atime: i64,
    pub st_atime_nsec: i64,
    pub st_mtime: i64,
    pub st_mtime_nsec: i64,
    pub st_ctime: i64,
    pub st_ctime_nsec: i64,
}

impl CStat {
    pub const fn from_kernel(stat: Stat) -> Self {
        Self {
            st_dev: 0,
            st_ino: stat.inode.raw(),
            st_mode: (file_type_mode(stat.kind) | (stat.mode & 0o777)) as u32,
            st_nlink: stat.links as u64,
            st_uid: stat.uid as u32,
            st_gid: stat.gid as u32,
            st_rdev: 0,
            st_size: stat.size as i64,
            st_blksize: 1,
            st_blocks: stat.size.div_ceil(512) as i64,
            st_atime: 0,
            st_atime_nsec: 0,
            st_mtime: 0,
            st_mtime_nsec: 0,
            st_ctime: 0,
            st_ctime_nsec: 0,
        }
    }
}

/// C-compatible `linux_dirent64`-style directory entry.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CDirEntry {
    pub d_ino: u64,
    pub d_off: i64,
    pub d_reclen: u16,
    pub d_type: u8,
    pub d_name: [u8; MAX_COMPONENT_BYTES + 1],
}

impl CDirEntry {
    pub const fn empty() -> Self {
        Self {
            d_ino: 0,
            d_off: 0,
            d_reclen: core::mem::size_of::<Self>() as u16,
            d_type: DT_UNKNOWN,
            d_name: [0; MAX_COMPONENT_BYTES + 1],
        }
    }

    pub fn from_kernel(entry: &DirEntry, offset_after_entry: usize) -> Self {
        let mut translated = Self::empty();
        translated.d_ino = entry.inode.raw();
        translated.d_off = offset_after_entry as i64;
        translated.d_type = dirent_type(entry.kind);
        let name = entry.name().as_bytes();
        let copy_len = core::cmp::min(name.len(), MAX_COMPONENT_BYTES);
        translated.d_name[..copy_len].copy_from_slice(&name[..copy_len]);
        translated
    }
}

pub const fn file_type_mode(kind: InodeKind) -> u16 {
    match kind {
        InodeKind::Directory => S_IFDIR,
        InodeKind::RegularFile => S_IFREG,
        InodeKind::Symlink => S_IFLNK,
        InodeKind::BlockDevice => S_IFBLK,
        InodeKind::CharDevice => S_IFCHR,
        InodeKind::Fifo => S_IFIFO,
        InodeKind::Socket => S_IFSOCK,
    }
}

pub const fn dirent_type(kind: InodeKind) -> u8 {
    match kind {
        InodeKind::Directory => DT_DIR,
        InodeKind::RegularFile => DT_REG,
        InodeKind::Symlink => DT_LNK,
        InodeKind::BlockDevice => DT_BLK,
        InodeKind::CharDevice => DT_CHR,
        InodeKind::Fifo => DT_FIFO,
        InodeKind::Socket => DT_SOCK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::fs::inode::{InodeId, InodeKind, Stat};

    #[test]
    fn libc_open_flags_translate_to_kernel_flags() {
        let flags = open_flags_from_libc(O_RDWR | O_CREAT | O_EXCL | O_TRUNC | O_CLOEXEC);
        assert!(flags.contains(OpenFlags::RDWR));
        assert!(flags.contains(OpenFlags::CREATE));
        assert!(flags.contains(OpenFlags::EXCLUSIVE));
        assert!(flags.contains(OpenFlags::TRUNCATE));
        assert!(flags.contains(OpenFlags::CLOSE_ON_EXEC));
    }

    #[test]
    fn libc_mode_and_stat_translate_to_abi_payloads() {
        let permissions = permissions_from_libc_mode(0o100755, 42, 7);
        assert_eq!(permissions.bits(), 0o755);
        assert_eq!(permissions.owner(), 42);
        assert_eq!(permissions.group(), 7);

        let stat = CStat::from_kernel(Stat {
            inode: InodeId::new(9),
            kind: InodeKind::Directory,
            size: 1025,
            mode: permissions.bits(),
            uid: permissions.owner(),
            gid: permissions.group(),
            links: 3,
        });
        assert_eq!(stat.st_ino, 9);
        assert_eq!(stat.st_mode, (S_IFDIR | 0o755) as u32);
        assert_eq!(stat.st_blocks, 3);
        assert_eq!(stat.st_nlink, 3);
    }
}
