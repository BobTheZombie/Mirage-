//! POSIX-style no-alloc runtime exports for Mirage user code.
//!
//! This module re-exports the C ABI filesystem shims implemented in
//! [`crate::libc`] so consumers can import a stdlib-shaped namespace while the
//! exported symbols remain single definitions in `libc`.

pub use crate::libc::{
    close, fstat, fsync, ftruncate, getdents64, lseek, mkdir, mkdirat, open, openat, read, rename,
    renameat, renameat2, stat, statx, unlink, unlinkat, write,
};

/// Filesystem ABI constants and C-compatible payloads shared with libc wrappers.
pub mod fs {
    pub use crate::kernel::fs::{
        CDirEntry, CStat, DT_BLK, DT_CHR, DT_DIR, DT_FIFO, DT_LNK, DT_REG, DT_SOCK, DT_UNKNOWN,
        F_OK, O_APPEND, O_CLOEXEC, O_CREAT, O_DIRECTORY, O_EXCL, O_NOFOLLOW, O_RDONLY, O_RDWR,
        O_TRUNC, O_WRONLY, R_OK, S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK, S_IFMT, S_IFREG,
        S_IFSOCK, W_OK, X_OK,
    };
}
