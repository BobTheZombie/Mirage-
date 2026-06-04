#![cfg_attr(all(not(test), not(feature = "qfs-std"), target_os = "none"), no_std)]
#![cfg_attr(
    all(not(test), not(feature = "qfs-std"), target_os = "none"),
    feature(alloc_error_handler)
)]

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

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
extern crate alloc;

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

/// Print formatted text to the early COM1 serial console.
///
/// The early console is a mechanism-only diagnostic path for boot milestones;
/// higher-level logging policy belongs above the kernel.
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {
        $crate::arch::x86_64::uart16550::early_print(::core::format_args!($($arg)*));
    };
}

/// Print a line to the early COM1 serial console.
#[macro_export]
macro_rules! kprintln {
    () => {
        $crate::kprint!("\n");
    };
    ($fmt:expr) => {
        $crate::kprint!(::core::concat!($fmt, "\n"));
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::kprint!(::core::concat!($fmt, "\n"), $($arg)*);
    };
}

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
use core::panic::PanicInfo;

#[cfg(all(not(test), not(feature = "qfs-std"), target_os = "none"))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::arch::x86_64::uart16550::early_print(::core::format_args!(
        "\n=== Mirage kernel panic ===\n"
    ));

    if let Some(location) = info.location() {
        crate::arch::x86_64::uart16550::early_print(::core::format_args!(
            "file: {}\nline: {}\n",
            location.file(),
            location.line()
        ));
    } else {
        crate::arch::x86_64::uart16550::early_print(::core::format_args!(
            "file: <unknown>\nline: <unknown>\n"
        ));
    }

    crate::arch::x86_64::uart16550::early_print(::core::format_args!(
        "message: {}\n",
        info.message()
    ));

    crate::arch::x86_64::panic_halt()
}
