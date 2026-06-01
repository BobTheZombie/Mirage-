//! Fixed-table mount registry for VFS backends.

use crate::kernel::fs::{path::Path, vfs::FileSystem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountError {
    TableFull,
    InvalidMountPoint,
    AlreadyMounted,
    NotMounted,
}

#[derive(Clone, Copy)]
pub struct Mount<'a> {
    pub mount_point: Path<'a>,
    pub filesystem: &'a dyn FileSystem,
}

pub struct MountTable<'a, const MAX: usize> {
    mounts: [Option<Mount<'a>>; MAX],
}

impl<'a, const MAX: usize> MountTable<'a, MAX> {
    pub const fn new() -> Self {
        Self {
            mounts: [None; MAX],
        }
    }

    pub fn clear(&mut self) {
        let mut idx = 0usize;
        while idx < MAX {
            self.mounts[idx] = None;
            idx += 1;
        }
    }

    pub fn mount(
        &mut self,
        mount_point: Path<'a>,
        filesystem: &'a dyn FileSystem,
    ) -> Result<(), MountError> {
        if !mount_point.is_root() && mount_point.as_str().ends_with('/') {
            return Err(MountError::InvalidMountPoint);
        }
        let mut idx = 0usize;
        while idx < MAX {
            if let Some(existing) = self.mounts[idx] {
                if existing.mount_point.as_str() == mount_point.as_str() {
                    return Err(MountError::AlreadyMounted);
                }
            }
            idx += 1;
        }

        idx = 0;
        while idx < MAX {
            if self.mounts[idx].is_none() {
                self.mounts[idx] = Some(Mount {
                    mount_point,
                    filesystem,
                });
                return Ok(());
            }
            idx += 1;
        }
        Err(MountError::TableFull)
    }

    pub fn resolve(&self, path: Path<'_>) -> Option<Mount<'a>> {
        let mut best: Option<Mount<'a>> = None;
        let mut idx = 0usize;
        while idx < MAX {
            if let Some(mount) = self.mounts[idx] {
                if path_matches_mount(path.as_str(), mount.mount_point.as_str()) {
                    if best
                        .map(|current| {
                            mount.mount_point.as_str().len() > current.mount_point.as_str().len()
                        })
                        .unwrap_or(true)
                    {
                        best = Some(mount);
                    }
                }
            }
            idx += 1;
        }
        best
    }

    pub fn unmount(&mut self, mount_point: Path<'_>) -> Result<(), MountError> {
        let mut idx = 0usize;
        while idx < MAX {
            if let Some(existing) = self.mounts[idx] {
                if existing.mount_point.as_str() == mount_point.as_str() {
                    self.mounts[idx] = None;
                    return Ok(());
                }
            }
            idx += 1;
        }
        Err(MountError::NotMounted)
    }
}

fn path_matches_mount(path: &str, mount: &str) -> bool {
    if mount == "/" {
        return true;
    }
    path == mount
        || path
            .strip_prefix(mount)
            .map(|rest| rest.starts_with('/'))
            .unwrap_or(false)
}
