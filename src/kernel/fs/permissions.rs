//! Compact Unix-like permission bits for kernel VFS objects.

pub const READ: u16 = 0o4;
pub const WRITE: u16 = 0o2;
pub const EXECUTE: u16 = 0o1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessMode {
    Read,
    Write,
    Execute,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub uid: u16,
    pub gid: u16,
    pub is_kernel: bool,
}

impl Credentials {
    pub const fn kernel() -> Self {
        Self {
            uid: 0,
            gid: 0,
            is_kernel: true,
        }
    }

    pub const fn user(uid: u16, gid: u16) -> Self {
        Self {
            uid,
            gid,
            is_kernel: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permissions {
    bits: u16,
    owner: u16,
    group: u16,
}

impl Permissions {
    pub const fn new(bits: u16, owner: u16, group: u16) -> Self {
        Self {
            bits: bits & 0o777,
            owner,
            group,
        }
    }

    pub const fn read_only() -> Self {
        Self::new(0o444, 0, 0)
    }

    pub const fn read_write() -> Self {
        Self::new(0o644, 0, 0)
    }

    pub const fn executable() -> Self {
        Self::new(0o755, 0, 0)
    }

    pub const fn bits(self) -> u16 {
        self.bits
    }

    pub const fn owner(self) -> u16 {
        self.owner
    }

    pub const fn group(self) -> u16 {
        self.group
    }

    pub const fn allows(self, credentials: Credentials, mode: AccessMode) -> bool {
        if credentials.is_kernel {
            return true;
        }

        let shift = if credentials.uid == self.owner {
            6
        } else if credentials.gid == self.group {
            3
        } else {
            0
        };
        let class_bits = (self.bits >> shift) & 0o7;
        match mode {
            AccessMode::Read => (class_bits & READ) != 0,
            AccessMode::Write => (class_bits & WRITE) != 0,
            AccessMode::Execute => (class_bits & EXECUTE) != 0,
            AccessMode::ReadWrite => (class_bits & (READ | WRITE)) == (READ | WRITE),
        }
    }
}
