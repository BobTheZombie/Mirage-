//! Open file handles and seek state.

use crate::kernel::fs::inode::InodeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl FileMode {
    pub const fn can_read(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    pub const fn can_write(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileHandle {
    inode: InodeId,
    cursor: u64,
    mode: FileMode,
}

impl FileHandle {
    pub const fn new(inode: InodeId, mode: FileMode) -> Self {
        Self {
            inode,
            cursor: 0,
            mode,
        }
    }

    pub const fn inode(self) -> InodeId {
        self.inode
    }

    pub const fn cursor(self) -> u64 {
        self.cursor
    }

    pub const fn mode(self) -> FileMode {
        self.mode
    }

    pub fn seek(&mut self, offset: u64) {
        self.cursor = offset;
    }

    pub fn advance(&mut self, bytes: usize) {
        self.cursor = self.cursor.saturating_add(bytes as u64);
    }
}
