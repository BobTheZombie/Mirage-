//! POSIX-style no-alloc runtime facade for Mirage user code.
//!
//! This module keeps a stable Rust-facing namespace over the focused libc ABI
//! modules. Exported C symbols remain single definitions in [`crate::libc`].

pub use crate::libc::stdlib as alloc;
pub use crate::libc::{dirent, errno, fcntl, pthread, socket, string, sys_stat, time, unistd};

pub use crate::libc::dirent::getdents64;
pub use crate::libc::fcntl::{open, openat};
pub use crate::libc::stdlib::{
    aligned_alloc, calloc, free, malloc, memalign, mmap, munmap, posix_memalign, realloc,
    reallocarray,
};
pub use crate::libc::string::{
    bcmp, bcopy, bzero, memchr, memcmp, memcpy, memmove, memset, strcat, strchr, strcmp, strcpy,
    strdup, strlen, strncat, strncmp, strncpy, strndup, strnlen, strrchr, strstr,
};
pub use crate::libc::sys_stat::{
    fstat, fsync, ftruncate, mkdir, mkdirat, rename, renameat, renameat2, stat, statx, unlink,
    unlinkat,
};
pub use crate::libc::time::clock_gettime;
pub use crate::libc::unistd::{
    close, getegid, geteuid, getgid, getpid, getppid, getuid, lseek, read, write,
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
