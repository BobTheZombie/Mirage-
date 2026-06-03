//! x86_64 programmed I/O port helpers used by legacy platform devices.

/// Read one byte from an I/O port.
#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    #[cfg(not(any(test, feature = "qfs-std")))]
    {
        let value: u8;
        core::arch::asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
        value
    }

    #[cfg(any(test, feature = "qfs-std"))]
    {
        let _ = port;
        0
    }
}

/// Write one byte to an I/O port.
#[inline(always)]
pub unsafe fn outb(port: u16, value: u8) {
    #[cfg(not(any(test, feature = "qfs-std")))]
    core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));

    #[cfg(any(test, feature = "qfs-std"))]
    {
        let _ = (port, value);
    }
}

/// Small delay for devices that require posted I/O operations to settle.
#[inline(always)]
pub fn io_wait() {
    unsafe { outb(0x80, 0) }
}
