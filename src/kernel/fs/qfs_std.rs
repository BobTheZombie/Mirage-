//! Kernel test compatibility re-exports for hosted QFS utilities.
//!
//! Host image operations live in [`crate::stdlib::qfs`] so userspace tools do
//! not depend on this private kernel filesystem module layout. This module is
//! kept as a narrow compatibility shim for kernel-facing tests that need the
//! [`crate::kernel::device::BlockStorageDevice`] implementation.

pub use crate::stdlib::qfs::StdQfsBlockDevice;
