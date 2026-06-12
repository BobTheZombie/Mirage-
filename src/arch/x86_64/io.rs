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

/// Read one 32-bit little-endian value from an I/O port.
#[inline(always)]
pub unsafe fn inl(port: u16) -> u32 {
    #[cfg(not(any(test, feature = "qfs-std")))]
    {
        let value: u32;
        core::arch::asm!("in eax, dx", out("eax") value, in("dx") port, options(nomem, nostack, preserves_flags));
        value
    }

    #[cfg(any(test, feature = "qfs-std"))]
    {
        let _ = port;
        0xffff_ffff
    }
}

/// Write one 32-bit little-endian value to an I/O port.
#[inline(always)]
pub unsafe fn outl(port: u16, value: u32) {
    #[cfg(not(any(test, feature = "qfs-std")))]
    core::arch::asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));

    #[cfg(any(test, feature = "qfs-std"))]
    {
        let _ = (port, value);
    }
}
