//! Architecture-local platform bring-up profiles.
//!
//! These profiles run in the lower kernel/architecture layer.  They detect and
//! label hardware required for safe boot, but they do not bind restartable
//! userspace driver services.

pub mod amd;
