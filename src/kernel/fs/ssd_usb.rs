//! Minimal SSD/USB-style filesystem over fixed in-memory directory entries.
//!
//! This is intentionally small: it models a block-backed removable or solid-state
//! volume without requiring heap allocation, dynamic path buffers, or recursive
//! directory walks. A real block driver can populate the fixed node table during
//! mount probing and then use the same [`FileSystem`] trait implementation.

use core::cmp::min;

use crate::kernel::{
    fs::{
        file::{File, FileHandle, OpenFlags},
        inode::{DirEntry, InodeId, InodeKind, InodeMetadata},
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
    links: u16,
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
            links: 1,
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
            links: 1,
            data: [0; MAX_FILE_BYTES],
        }
    }

    fn metadata(&self) -> InodeMetadata {
        InodeMetadata::with_links(
            self.inode,
            self.kind,
            self.size as u64,
            self.permissions,
            self.links,
        )
    }

    fn name_matches(&self, name: &str) -> bool {
        let bytes = name.as_bytes();
        self.name_len == bytes.len() && self.name[..self.name_len] == *bytes
    }

    fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
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
        self.node_index_by_parent_name(parent, name)
            .and_then(|idx| self.nodes[idx])
    }

    fn node_index_by_parent_name(&self, parent: InodeId, name: &str) -> Option<usize> {
        let mut idx = 0usize;
        while idx < MAX_VOLUME_NODES {
            if let Some(node) = self.nodes[idx] {
                if node.parent == parent && node.name_matches(name) {
                    return Some(idx);
                }
            }
            idx += 1;
        }
        None
    }

    fn resolve_parent<'a>(&self, path: Path<'a>) -> Result<(InodeId, &'a str), FsError> {
        if path.is_root() {
            return Err(FsError::InvalidArgument);
        }
        let mut components = path.components();
        let mut current = self.node_by_inode(InodeId::ROOT).ok_or(FsError::NotFound)?;
        let mut component = components.next().ok_or(FsError::InvalidArgument)?;
        loop {
            if let Some(next) = components.next() {
                if current.kind != InodeKind::Directory {
                    return Err(FsError::NotDirectory);
                }
                current = self
                    .find_child(current.inode, component)
                    .ok_or(FsError::NotFound)?;
                component = next;
            } else {
                if current.kind != InodeKind::Directory {
                    return Err(FsError::NotDirectory);
                }
                return Ok((current.inode, component));
            }
        }
    }

    fn create_node(
        &mut self,
        parent: InodeId,
        name: &str,
        kind: InodeKind,
        permissions: Permissions,
        initial: &[u8],
    ) -> Result<InodeId, FsError> {
        if name.is_empty() || name.len() > MAX_NAME_BYTES {
            return Err(FsError::InvalidPath(PathError::ComponentTooLong));
        }
        if self.find_child(parent, name).is_some() {
            return Err(FsError::AlreadyExists);
        }
        if self.node_by_inode(parent).map(|node| node.kind) != Some(InodeKind::Directory) {
            return Err(FsError::NotDirectory);
        }
        let slot = self.free_slot().ok_or(FsError::NoSpace)?;
        let mut node = Node::empty();
        node.inode = InodeId::new(self.next_inode);
        node.parent = parent;
        node.kind = kind;
        node.name_len = name.len();
        node.name[..name.len()].copy_from_slice(name.as_bytes());
        node.permissions = permissions;
        node.size = min(initial.len(), MAX_FILE_BYTES);
        node.data[..node.size].copy_from_slice(&initial[..node.size]);
        self.next_inode = self.next_inode.saturating_add(1);
        self.nodes[slot] = Some(node);
        Ok(node.inode)
    }

    fn is_empty_directory(&self, inode: InodeId) -> bool {
        let mut idx = 0usize;
        while idx < MAX_VOLUME_NODES {
            if let Some(node) = self.nodes[idx] {
                if node.parent == inode && node.inode != inode {
                    return false;
                }
            }
            idx += 1;
        }
        true
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
        self.state
            .lock()
            .create_node(parent, name, InodeKind::RegularFile, permissions, initial)
    }
}

impl FileSystem for SsdUsbFileSystem {
    fn root_inode(&self) -> InodeId {
        InodeId::ROOT
    }

    fn lookup(&self, path: Path<'_>) -> Result<InodeMetadata, FsError> {
        Ok(self.state.lock().resolve_inode(path)?.metadata())
    }

    fn lookup_inode(&self, inode: InodeId) -> Result<InodeMetadata, FsError> {
        self.state
            .lock()
            .node_by_inode(inode)
            .map(|node| node.metadata())
            .ok_or(FsError::NotFound)
    }

    fn open(
        &self,
        path: Path<'_>,
        flags: OpenFlags,
        credentials: Credentials,
    ) -> Result<File, FsError> {
        let metadata = self.lookup(path)?;
        if flags.contains(OpenFlags::DIRECTORY) && metadata.kind != InodeKind::Directory {
            return Err(FsError::NotDirectory);
        }
        if metadata.kind == InodeKind::Directory && flags.access_mode().can_write() {
            return Err(FsError::IsDirectory);
        }
        let access = match flags.access_mode() {
            crate::kernel::fs::file::FileMode::ReadOnly => {
                crate::kernel::fs::permissions::AccessMode::Read
            }
            crate::kernel::fs::file::FileMode::WriteOnly => {
                crate::kernel::fs::permissions::AccessMode::Write
            }
            crate::kernel::fs::file::FileMode::ReadWrite => {
                crate::kernel::fs::permissions::AccessMode::ReadWrite
            }
        };
        if !metadata.permissions.allows(credentials, access) {
            return Err(FsError::PermissionDenied);
        }
        if self.read_only && flags.access_mode().can_write() {
            return Err(FsError::ReadOnly);
        }
        let mut file = File::with_flags(metadata.id, flags);
        if flags.contains(OpenFlags::APPEND) {
            file.seek(metadata.size);
        }
        if flags.contains(OpenFlags::TRUNCATE) && flags.access_mode().can_write() {
            self.truncate(path, 0, credentials)?;
        }
        Ok(file)
    }

    fn pread(&self, handle: &FileHandle, buffer: &mut [u8], offset: u64) -> Result<usize, FsError> {
        if !handle.mode().can_read() {
            return Err(FsError::PermissionDenied);
        }
        let state = self.state.lock();
        let node = state
            .node_by_inode(handle.inode())
            .ok_or(FsError::InvalidHandle)?;
        if node.kind == InodeKind::Directory {
            return Err(FsError::IsDirectory);
        }
        if node.kind != InodeKind::RegularFile && node.kind != InodeKind::Symlink {
            return Err(FsError::Unsupported);
        }
        let offset = min(offset as usize, node.size);
        let available = node.size - offset;
        let to_copy = min(available, buffer.len());
        buffer[..to_copy].copy_from_slice(&node.data[offset..offset + to_copy]);
        Ok(to_copy)
    }

    fn pwrite(&self, handle: &FileHandle, data: &[u8], offset: u64) -> Result<usize, FsError> {
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
        if node.kind == InodeKind::Directory {
            return Err(FsError::IsDirectory);
        }
        if node.kind != InodeKind::RegularFile && node.kind != InodeKind::Symlink {
            return Err(FsError::Unsupported);
        }
        let offset = min(offset as usize, MAX_FILE_BYTES);
        let to_copy = min(MAX_FILE_BYTES - offset, data.len());
        if to_copy == 0 && !data.is_empty() {
            return Err(FsError::NoSpace);
        }
        node.data[offset..offset + to_copy].copy_from_slice(&data[..to_copy]);
        node.size = node.size.max(offset + to_copy);
        state.nodes[index] = Some(node);
        Ok(to_copy)
    }

    fn mkdir(
        &self,
        path: Path<'_>,
        mode: Permissions,
        _credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let mut state = self.state.lock();
        let (parent, name) = state.resolve_parent(path)?;
        state.create_node(parent, name, InodeKind::Directory, mode, &[])?;
        Ok(())
    }

    fn rmdir(&self, path: Path<'_>, _credentials: Credentials) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let mut state = self.state.lock();
        let node = state.resolve_inode(path)?;
        if node.kind != InodeKind::Directory {
            return Err(FsError::NotDirectory);
        }
        if node.inode == InodeId::ROOT || !state.is_empty_directory(node.inode) {
            return Err(FsError::Busy);
        }
        let index = state
            .node_index_by_inode(node.inode)
            .ok_or(FsError::NotFound)?;
        state.nodes[index] = None;
        Ok(())
    }

    fn unlink(&self, path: Path<'_>, _credentials: Credentials) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let mut state = self.state.lock();
        let node = state.resolve_inode(path)?;
        if node.kind == InodeKind::Directory {
            return Err(FsError::IsDirectory);
        }
        let index = state
            .node_index_by_inode(node.inode)
            .ok_or(FsError::NotFound)?;
        state.nodes[index] = None;
        Ok(())
    }

    fn rename(
        &self,
        old_path: Path<'_>,
        new_path: Path<'_>,
        _credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let mut state = self.state.lock();
        let (old_parent, old_name) = state.resolve_parent(old_path)?;
        let index = state
            .node_index_by_parent_name(old_parent, old_name)
            .ok_or(FsError::NotFound)?;
        let (new_parent, new_name) = state.resolve_parent(new_path)?;
        if new_name.len() > MAX_NAME_BYTES {
            return Err(FsError::InvalidPath(PathError::ComponentTooLong));
        }
        if state.find_child(new_parent, new_name).is_some() {
            return Err(FsError::AlreadyExists);
        }
        let mut renamed = state.nodes[index].ok_or(FsError::NotFound)?;
        renamed.parent = new_parent;
        renamed.name = [0; MAX_NAME_BYTES];
        renamed.name_len = new_name.len();
        renamed.name[..new_name.len()].copy_from_slice(new_name.as_bytes());
        state.nodes[index] = Some(renamed);
        Ok(())
    }

    fn link(
        &self,
        old_path: Path<'_>,
        new_path: Path<'_>,
        _credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let mut state = self.state.lock();
        let source = state.resolve_inode(old_path)?;
        if source.kind == InodeKind::Directory {
            return Err(FsError::IsDirectory);
        }
        let (parent, name) = state.resolve_parent(new_path)?;
        if name.is_empty() || name.len() > MAX_NAME_BYTES {
            return Err(FsError::InvalidPath(PathError::ComponentTooLong));
        }
        if state.find_child(parent, name).is_some() {
            return Err(FsError::AlreadyExists);
        }
        let slot = state.free_slot().ok_or(FsError::NoSpace)?;
        let mut alias = source;
        alias.parent = parent;
        alias.name = [0; MAX_NAME_BYTES];
        alias.name_len = name.len();
        alias.name[..name.len()].copy_from_slice(name.as_bytes());
        alias.links = alias.links.saturating_add(1);
        state.nodes[slot] = Some(alias);
        Ok(())
    }

    fn symlink(
        &self,
        target: Path<'_>,
        link_path: Path<'_>,
        _credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let mut state = self.state.lock();
        let (parent, name) = state.resolve_parent(link_path)?;
        state.create_node(
            parent,
            name,
            InodeKind::Symlink,
            Permissions::read_write(),
            target.as_str().as_bytes(),
        )?;
        Ok(())
    }

    fn readlink(&self, path: Path<'_>, buffer: &mut [u8]) -> Result<usize, FsError> {
        let state = self.state.lock();
        let node = state.resolve_inode(path)?;
        if node.kind != InodeKind::Symlink {
            return Err(FsError::InvalidArgument);
        }
        let to_copy = min(node.size, buffer.len());
        buffer[..to_copy].copy_from_slice(&node.data[..to_copy]);
        Ok(to_copy)
    }

    fn chmod(&self, path: Path<'_>, mode: u16, _credentials: Credentials) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let mut state = self.state.lock();
        let node = state.resolve_inode(path)?;
        let index = state
            .node_index_by_inode(node.inode)
            .ok_or(FsError::NotFound)?;
        let mut updated = state.nodes[index].ok_or(FsError::NotFound)?;
        updated.permissions = Permissions::new(
            mode,
            updated.permissions.owner(),
            updated.permissions.group(),
        );
        state.nodes[index] = Some(updated);
        Ok(())
    }

    fn chown(
        &self,
        path: Path<'_>,
        uid: u16,
        gid: u16,
        _credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let mut state = self.state.lock();
        let node = state.resolve_inode(path)?;
        let index = state
            .node_index_by_inode(node.inode)
            .ok_or(FsError::NotFound)?;
        let mut updated = state.nodes[index].ok_or(FsError::NotFound)?;
        updated.permissions = Permissions::new(updated.permissions.bits(), uid, gid);
        state.nodes[index] = Some(updated);
        Ok(())
    }

    fn truncate(
        &self,
        path: Path<'_>,
        size: u64,
        _credentials: Credentials,
    ) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        if size as usize > MAX_FILE_BYTES {
            return Err(FsError::NoSpace);
        }
        let mut state = self.state.lock();
        let node = state.resolve_inode(path)?;
        if node.kind == InodeKind::Directory {
            return Err(FsError::IsDirectory);
        }
        let index = state
            .node_index_by_inode(node.inode)
            .ok_or(FsError::NotFound)?;
        let mut updated = state.nodes[index].ok_or(FsError::NotFound)?;
        let new_size = size as usize;
        if new_size > updated.size {
            updated.data[updated.size..new_size].fill(0);
        }
        updated.size = new_size;
        state.nodes[index] = Some(updated);
        Ok(())
    }

    fn readdir(
        &self,
        path: Path<'_>,
        offset: usize,
        entries: &mut [DirEntry],
    ) -> Result<usize, FsError> {
        let state = self.state.lock();
        let directory = state.resolve_inode(path)?;
        if directory.kind != InodeKind::Directory {
            return Err(FsError::NotDirectory);
        }
        let mut seen = 0usize;
        let mut written = 0usize;
        let mut idx = 0usize;
        while idx < MAX_VOLUME_NODES && written < entries.len() {
            if let Some(node) = state.nodes[idx] {
                if node.parent == directory.inode && node.inode != directory.inode {
                    if seen >= offset {
                        entries[written] = DirEntry::new(node.inode, node.kind, node.name_str())?;
                        written += 1;
                    }
                    seen += 1;
                }
            }
            idx += 1;
        }
        Ok(written)
    }
}
