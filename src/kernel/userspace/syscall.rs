//! Minimal Spider-rs syscall ABI notes and decoders.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserspaceSyscall {
    Exit = 1,
    Write = 2,
    Yield = 3,
    GetPid = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyscallDecodeError {
    Unknown,
}

pub const X86_64_ARGUMENT_REGISTERS: [&str; 6] = ["rdi", "rsi", "rdx", "r10", "r8", "r9"];

pub const fn decode(number: u64) -> Result<UserspaceSyscall, SyscallDecodeError> {
    match number {
        1 => Ok(UserspaceSyscall::Exit),
        2 => Ok(UserspaceSyscall::Write),
        3 => Ok(UserspaceSyscall::Yield),
        4 => Ok(UserspaceSyscall::GetPid),
        _ => Err(SyscallDecodeError::Unknown),
    }
}

pub const fn user_buffer_in_bounds(ptr: u64, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    match ptr.checked_add(len as u64) {
        Some(end) => ptr < 0x0000_8000_0000_0000 && end <= 0x0000_8000_0000_0000,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syscall_number_decoding_matches_spider_abi() {
        assert_eq!(decode(1), Ok(UserspaceSyscall::Exit));
        assert_eq!(decode(2), Ok(UserspaceSyscall::Write));
        assert_eq!(decode(3), Ok(UserspaceSyscall::Yield));
        assert_eq!(decode(4), Ok(UserspaceSyscall::GetPid));
        assert_eq!(decode(99), Err(SyscallDecodeError::Unknown));
    }

    #[test]
    fn write_syscall_user_pointer_bounds_reject_kernel_addresses() {
        assert!(user_buffer_in_bounds(0x1000, 4));
        assert!(!user_buffer_in_bounds(0xffff_8000_0000_0000, 4));
    }
}
