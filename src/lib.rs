#![cfg_attr(not(test), no_std)]

//! Mirage is a conceptual 64-bit, Rust-based GNU/Mirage kernel organized around a
//! mechanism/policy split.
//!
//! * The **mechanism-only kernel layer** provides CPU scheduling primitives, process
//!   lifecycle mechanics, message-based IPC, filesystem mechanisms and syscall entry
//!   points without making POSIX or Linux conventions part of the internal architecture.
//! * The **supervisor and security broker layers** own service policy, recovery,
//!   signed-manifest validation and security adjudication through isolation domains,
//!   credentials, capabilities and message authorization. Supervised driver services are
//!   the preferred driver model.
//!
//! The code in this crate is designed to illustrate that GNU/Mirage structure without relying
//! on the standard library. POSIX/GNU compatibility is documented as an external ABI surface;
//! internally, QFS is treated as the native indexed object filesystem and boot assumptions are
//! expressed as a signed boot module set rather than compatibility-driven startup conventions.

#[cfg(feature = "qfs-std")]
extern crate std;

pub mod arch;
pub mod boot;
pub mod kernel;
#[cfg(not(feature = "qfs-std"))]
pub mod libc;
#[cfg(not(feature = "qfs-std"))]
pub mod librust;
pub mod stdlib;
pub mod subkernel;
pub mod supervisor;

#[cfg(not(any(test, feature = "qfs-std")))]
use core::panic::PanicInfo;

#[cfg(not(any(test, feature = "qfs-std")))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    crate::arch::x86_64::panic_halt()
}
