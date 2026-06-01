//! Minimal SSD/USB-style filesystem over fixed in-memory directory entries.
//!
//! This is intentionally small: it models a block-backed removable or solid-state
//! volume without requiring heap allocation, dynamic path buffers, or recursive
//! directory walks. A real block driver can populate the fixed node table during
//! mount probing and then use the same [`FileSystem`] trait implementation.

use core::cmp::min;

use crate::kernel::{
    fs::{
        file::{FileHandle, FileMode},
        inode::{InodeId, InodeKind, InodeMetadata},
        path::{Path, PathError},
        permissions::{Credentials, Permissions},
        vfs::{FileSystem, FsError},
    },
    sync::SpinLock,
};

pub const MAX_VOLUME_NODES: usize = 16;
pub const MAX_FILE_BYTES: usize = 4096;
pub const MAX_NAME_BYTES: usize = 24;

#[derive(Clone, Copy)]
struct Node {
    inode: InodeId,
    parent: InodeId,
    kind: InodeKind,
    name: [u8; MAX_NAME_BYTES],
    name_len: usize,
    size: usize,
    permissions: Permissions,
    data: [u8; MAX_FILE_BYTES],
}

impl Node {
    const fn empty() -> Self {
        Self {
            inode: InodeId::new(0),
            parent: InodeId::new(0),
            kind: InodeKind::RegularFile,
            name: [0; MAX_NAME_BYTES],
            name_len: 0,
            size: 0,
            permissions: Permissions::read_only(),
            data: [0; MAX_FILE_BYTES],
        }
    }

    const fn root() -> Self {
        Self {
            inode: InodeId::ROOT,
            parent: InodeId::ROOT,
            kind: InodeKind::Directory,
            name: [0; MAX_NAME_BYTES],
            name_len: 0,
            size: 0,
            permissions: Permissions::executable(),
            data: [0; MAX_FILE_BYTES],
        }
    }

    fn metadata(&self) -> InodeMetadata {
        InodeMetadata::new(self.inode, self.kind, self.size as u64, self.permissions)
    }

    fn name_matches(&self, name: &str) -> bool {
        let bytes = name.as_bytes();
        self.name_len == bytes.len() && self.name[..self.name_len] == *bytes
    }
}

struct VolumeState {
    nodes: [Option<Node>; MAX_VOLUME_NODES],
    next_inode: u64,
}

impl VolumeState {
    const fn new() -> Self {
        let mut nodes = [None; MAX_VOLUME_NODES];
        nodes[0] = Some(Node::root());
        Self {
            nodes,
            next_inode: 2,
        }
    }

    fn free_slot(&self) -> Option<usize> {
        let mut idx = 0usize;
        while idx < MAX_VOLUME_NODES {
            if self.nodes[idx].is_none() {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    fn node_by_inode(&self, inode: InodeId) -> Option<Node> {
        let mut idx = 0usize;
        while idx < MAX_VOLUME_NODES {
            if let Some(node) = self.nodes[idx] {
                if node.inode == inode {
                    return Some(node);
                }
            }
            idx += 1;
        }
        None
    }

    fn node_index_by_inode(&self, inode: InodeId) -> Option<usize> {
        let mut idx = 0usize;
        while idx < MAX_VOLUME_NODES {
            if let Some(node) = self.nodes[idx] {
                if node.inode == inode {
                    return Some(idx);
                }
            }
            idx += 1;
        }
        None
    }

    fn find_child(&self, parent: InodeId, name: &str) -> Option<Node> {
        let mut idx = 0usize;
        while idx < MAX_VOLUME_NODES {
            if let Some(node) = self.nodes[idx] {
                if node.parent == parent && node.name_matches(name) {
                    return Some(node);
                }
            }
            idx += 1;
        }
        None
    }

    fn resolve_inode(&self, path: Path<'_>) -> Result<Node, FsError> {
        if path.is_root() {
            return self.node_by_inode(InodeId::ROOT).ok_or(FsError::NotFound);
        }
        let mut current = self.node_by_inode(InodeId::ROOT).ok_or(FsError::NotFound)?;
        let mut components = path.components();
        while let Some(component) = components.next() {
            if current.kind != InodeKind::Directory {
                return Err(FsError::NotDirectory);
            }
            current = self
                .find_child(current.inode, component)
                .ok_or(FsError::NotFound)?;
        }
        Ok(current)
    }
}

pub struct SsdUsbFileSystem {
    state: SpinLock<VolumeState>,
    read_only: bool,
}

impl SsdUsbFileSystem {
    pub const fn new(read_only: bool) -> Self {
        Self {
            state: SpinLock::new(VolumeState::new()),
            read_only,
        }
    }

    pub fn create_file(
        &self,
        parent: InodeId,
        name: &str,
        permissions: Permissions,
        initial: &[u8],
    ) -> Result<InodeId, FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        if name.is_empty() || name.len() > MAX_NAME_BYTES {
            return Err(FsError::InvalidPath(PathError::ComponentTooLong));
        }

        let mut state = self.state.lock();
        if state.find_child(parent, name).is_some() {
            return Err(FsError::AlreadyExists);
        }
        if state.node_by_inode(parent).map(|node| node.kind) != Some(InodeKind::Directory) {
            return Err(FsError::NotDirectory);
        }

        let slot = state.free_slot().ok_or(FsError::NoSpace)?;
        let mut node = Node::empty();
        node.inode = InodeId::new(state.next_inode);
        node.parent = parent;
        node.kind = InodeKind::RegularFile;
        node.name_len = name.len();
        node.name[..name.len()].copy_from_slice(name.as_bytes());
        node.permissions = permissions;
        node.size = min(initial.len(), MAX_FILE_BYTES);
        node.data[..node.size].copy_from_slice(&initial[..node.size]);
        state.next_inode = state.next_inode.saturating_add(1);
        state.nodes[slot] = Some(node);
        Ok(node.inode)
    }
}

impl FileSystem for SsdUsbFileSystem {
    fn root_inode(&self) -> InodeId {
        InodeId::ROOT
    }

    fn lookup(&self, path: Path<'_>) -> Result<InodeMetadata, FsError> {
        Ok(self.state.lock().resolve_inode(path)?.metadata())
    }

    fn open(
        &self,
        path: Path<'_>,
        mode: FileMode,
        credentials: Credentials,
    ) -> Result<FileHandle, FsError> {
        let metadata = self.lookup(path)?;
        if metadata.kind == InodeKind::Directory {
            return Err(FsError::Unsupported);
        }
        let access = match mode {
            FileMode::ReadOnly => crate::kernel::fs::permissions::AccessMode::Read,
            FileMode::WriteOnly => crate::kernel::fs::permissions::AccessMode::Write,
            FileMode::ReadWrite => crate::kernel::fs::permissions::AccessMode::ReadWrite,
        };
        if !metadata.permissions.allows(credentials, access) {
            return Err(FsError::PermissionDenied);
        }
        if self.read_only && mode.can_write() {
            return Err(FsError::ReadOnly);
        }
        Ok(FileHandle::new(metadata.id, mode))
    }

    fn read(&self, handle: &mut FileHandle, buffer: &mut [u8]) -> Result<usize, FsError> {
        if !handle.mode().can_read() {
            return Err(FsError::PermissionDenied);
        }
        let state = self.state.lock();
        let node = state
            .node_by_inode(handle.inode())
            .ok_or(FsError::InvalidHandle)?;
        if node.kind != InodeKind::RegularFile {
            return Err(FsError::Unsupported);
        }
        let offset = min(handle.cursor() as usize, node.size);
        let available = node.size - offset;
        let to_copy = min(available, buffer.len());
        buffer[..to_copy].copy_from_slice(&node.data[offset..offset + to_copy]);
        handle.advance(to_copy);
        Ok(to_copy)
    }

    fn write(&self, handle: &mut FileHandle, data: &[u8]) -> Result<usize, FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        if !handle.mode().can_write() {
            return Err(FsError::PermissionDenied);
        }
        let mut state = self.state.lock();
        let index = state
            .node_index_by_inode(handle.inode())
            .ok_or(FsError::InvalidHandle)?;
        let mut node = state.nodes[index].ok_or(FsError::InvalidHandle)?;
        if node.kind != InodeKind::RegularFile {
            return Err(FsError::Unsupported);
        }
        let offset = min(handle.cursor() as usize, MAX_FILE_BYTES);
        let to_copy = min(MAX_FILE_BYTES - offset, data.len());
        if to_copy == 0 {
            return Err(FsError::NoSpace);
        }
        node.data[offset..offset + to_copy].copy_from_slice(&data[..to_copy]);
        node.size = node.size.max(offset + to_copy);
        state.nodes[index] = Some(node);
        handle.advance(to_copy);
        Ok(to_copy)
    }
}
