//! Userspace launch scaffolding for the GNU/Mirage PID 1 handoff.
//!
//! This module is intentionally honest about the current milestone: it can
//! validate static ELF64 images and build deterministic initial-stack metadata,
//! but the architecture backend still needs the real ring-3 entry path before
//! Spider-rs may be marked `Online`.

pub mod abi;
pub mod elf_loader;
pub mod memory;
pub mod syscall;

pub use elf_loader::{load_elf_from_file, validate_elf64, LoadError, LoadedProgram};
pub use memory::{
    allocate_user_stack, create_user_address_space, map_user_region, MmError, PhysAddr,
    UserMapFlags, UserStack, VirtAddr,
};
