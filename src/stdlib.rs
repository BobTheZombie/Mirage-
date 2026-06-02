//! POSIX-style no-alloc runtime exports for Mirage user code.
//!
//! This module re-exports the C ABI filesystem shims implemented in
//! [`crate::libc`] so consumers can import a stdlib-shaped namespace while the
//! exported symbols remain single definitions in `libc`.

pub use crate::libc::{
    close, fstat, fsync, ftruncate, getdents64, lseek, mkdir, mkdirat, open, openat, read, rename,
    renameat, renameat2, stat, statx, unlink, unlinkat, write,
};
