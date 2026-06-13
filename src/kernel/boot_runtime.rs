//! Immutable RuntimeVfs for Spider Runtime for early userspace PID 1 availability.
//!
//! The image format is deliberately simple and no-heap friendly: a fixed binary
//! header followed by file entries and byte payloads.  It is not a normal root
//! filesystem; it is kernel-owned, read-only, and mounted at `/spider-rt`.

use crate::arch::x86_64::boot::{BootModules, VirtualAddress};
use crate::kernel::block::BlockError;
use crate::kernel::mmio::PhysAddr;

pub const BOOTRT_MAGIC: &[u8; 8] = b"MBRTFS\0\x01";
pub const BOOTRT_ENTRY_PATH_CAP: usize = 64;
pub const BOOTRT_MAX_FILES: usize = 16;
pub const BOOTRT_ENTRY_SIZE: usize = 128;
pub const BOOTRT_HEADER_SIZE: usize = 64;
pub const BOOTRT_ENTRY: &str = "/sbin/spider-rs";
pub const BOOTRT_MOUNTED_ENTRY: &str = "/spider-rt/sbin/spider-rs";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootRuntime {
    pub image_start: PhysAddr,
    pub image_len: usize,
    pub verified: bool,
    pub mounted: bool,
    pub immutable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootRuntimeFile {
    pub path: [u8; BOOTRT_ENTRY_PATH_CAP],
    pub path_len: usize,
    pub offset: usize,
    pub size: usize,
    pub hash: u32,
    pub entry: bool,
}

impl BootRuntimeFile {
    pub const fn empty() -> Self {
        Self {
            path: [0; BOOTRT_ENTRY_PATH_CAP],
            path_len: 0,
            offset: 0,
            size: 0,
            hash: 0,
            entry: false,
        }
    }

    pub fn path(&self) -> &str {
        core::str::from_utf8(&self.path[..self.path_len]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootRuntimeManifest {
    pub name: [u8; 32],
    pub version: [u8; 16],
    pub entry: [u8; BOOTRT_ENTRY_PATH_CAP],
    pub entry_len: usize,
    pub files: [Option<BootRuntimeFile>; BOOTRT_MAX_FILES],
    pub file_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootRuntimeRamFs {
    image: &'static [u8],
    manifest: BootRuntimeManifest,
}

impl BootRuntimeRamFs {
    pub fn mount(image: &'static [u8]) -> Result<(BootRuntime, Self), BlockError> {
        let manifest = parse_manifest(image)?;
        let entry = manifest.entry_path();
        if find_file_in_manifest(&manifest, entry).is_none() {
            return Err(BlockError::NotFound);
        }
        Ok((
            BootRuntime {
                image_start: PhysAddr(image.as_ptr() as u64),
                image_len: image.len(),
                verified: true,
                mounted: true,
                immutable: true,
            },
            Self { image, manifest },
        ))
    }

    pub const fn manifest(&self) -> &BootRuntimeManifest {
        &self.manifest
    }

    pub fn lookup(&self, path: &str) -> Option<BootRuntimeFile> {
        let normalized = normalize_bootrt_path(path);
        find_file_in_manifest(&self.manifest, normalized)
    }

    pub fn read(&self, path: &str, offset: usize, out: &mut [u8]) -> Result<usize, BlockError> {
        let file = self.lookup(path).ok_or(BlockError::NotFound)?;
        if offset > file.size {
            return Err(BlockError::OutOfBounds);
        }
        let read_len = core::cmp::min(out.len(), file.size - offset);
        let start = file
            .offset
            .checked_add(offset)
            .ok_or(BlockError::OutOfBounds)?;
        let end = start.checked_add(read_len).ok_or(BlockError::OutOfBounds)?;
        if end > self.image.len() {
            return Err(BlockError::OutOfBounds);
        }
        out[..read_len].copy_from_slice(&self.image[start..end]);
        Ok(read_len)
    }

    pub fn write(&self, _path: &str, _offset: usize, _data: &[u8]) -> Result<(), BlockError> {
        Err(BlockError::ReadOnly)
    }
}

impl BootRuntimeManifest {
    pub fn entry_path(&self) -> &str {
        core::str::from_utf8(&self.entry[..self.entry_len]).unwrap_or(BOOTRT_ENTRY)
    }
}

pub fn find_boot_runtime_module(modules: BootModules) -> Option<&'static [u8]> {
    let mut index = 0;
    while index < modules.len() {
        if let Some(module) = modules.module(index) {
            if module_matches_bootrt(module.path.as_bytes())
                || module_matches_bootrt(module.command_line.as_bytes())
            {
                return module_slice(module.base, module.size);
            }
        }
        index += 1;
    }
    None
}

fn module_matches_bootrt(bytes: &[u8]) -> bool {
    contains(bytes, b"spider-rt")
        || contains(bytes, b"bootrt")
        || contains(bytes, b"mirage-boot-runtime")
}

fn module_slice(base: VirtualAddress, size: u64) -> Option<&'static [u8]> {
    if base.0 == 0 || size == 0 || size > usize::MAX as u64 {
        return None;
    }
    Some(unsafe { core::slice::from_raw_parts(base.0 as *const u8, size as usize) })
}

pub fn parse_manifest(image: &[u8]) -> Result<BootRuntimeManifest, BlockError> {
    if image.len() < BOOTRT_HEADER_SIZE || &image[0..8] != BOOTRT_MAGIC {
        return Err(BlockError::InvalidSignature);
    }
    let file_count = le_u32(image, 8) as usize;
    if file_count == 0 || file_count > BOOTRT_MAX_FILES {
        return Err(BlockError::InvalidSignature);
    }
    let entries_offset = le_u32(image, 12) as usize;
    let name_len = core::cmp::min(image[16] as usize, 32);
    let version_len = core::cmp::min(image[17] as usize, 16);
    let entry_len = core::cmp::min(image[18] as usize, BOOTRT_ENTRY_PATH_CAP);
    if entries_offset
        .checked_add(file_count * BOOTRT_ENTRY_SIZE)
        .map_or(true, |end| end > image.len())
    {
        return Err(BlockError::OutOfBounds);
    }
    let mut manifest = BootRuntimeManifest {
        name: [0; 32],
        version: [0; 16],
        entry: [0; BOOTRT_ENTRY_PATH_CAP],
        entry_len,
        files: [None; BOOTRT_MAX_FILES],
        file_count: 0,
    };
    manifest.name[..name_len].copy_from_slice(&image[20..20 + name_len]);
    manifest.version[..version_len].copy_from_slice(&image[52..52 + version_len]);
    manifest.entry[..entry_len]
        .copy_from_slice(&image[BOOTRT_HEADER_SIZE..BOOTRT_HEADER_SIZE + entry_len]);

    for idx in 0..file_count {
        let off = entries_offset + idx * BOOTRT_ENTRY_SIZE;
        let path_len = core::cmp::min(image[off] as usize, BOOTRT_ENTRY_PATH_CAP);
        let flags = image[off + 1];
        let file_offset = le_u32(image, off + 4) as usize;
        let size = le_u32(image, off + 8) as usize;
        let hash = le_u32(image, off + 12);
        if file_offset
            .checked_add(size)
            .map_or(true, |end| end > image.len())
        {
            return Err(BlockError::OutOfBounds);
        }
        let mut hash_image = [0u8; BOOTRT_ENTRY_SIZE];
        hash_image.copy_from_slice(&image[off..off + BOOTRT_ENTRY_SIZE]);
        hash_image[12..16].copy_from_slice(&0u32.to_le_bytes());
        if overlaps(file_offset, size, off, BOOTRT_ENTRY_SIZE) {
            for pos in 0..size {
                let absolute = file_offset + pos;
                if absolute >= off && absolute < off + BOOTRT_ENTRY_SIZE {
                    let entry_pos = absolute - off;
                    if image[absolute] != hash_image[entry_pos] {
                        return Err(BlockError::OutOfBounds);
                    }
                }
            }
        }
        if crc32(&image[file_offset..file_offset + size]) != hash {
            return Err(BlockError::Crc);
        }
        let mut file = BootRuntimeFile::empty();
        file.path_len = path_len;
        file.path[..path_len].copy_from_slice(&image[off + 16..off + 16 + path_len]);
        file.offset = file_offset;
        file.size = size;
        file.hash = hash;
        file.entry = (flags & 1) != 0;
        manifest.files[idx] = Some(file);
        manifest.file_count += 1;
    }
    Ok(manifest)
}

fn find_file_in_manifest(manifest: &BootRuntimeManifest, path: &str) -> Option<BootRuntimeFile> {
    let normalized = normalize_bootrt_path(path);
    for file in manifest.files.iter().flatten() {
        if file.path() == normalized {
            return Some(*file);
        }
    }
    None
}

fn normalize_bootrt_path(path: &str) -> &str {
    path.strip_prefix("/spider-rt")
        .or_else(|| path.strip_prefix("/bootrt"))
        .unwrap_or(path)
}

pub fn crc32(bytes: &[u8]) -> u32 {
    crate::kernel::partition::crc32(bytes)
}

fn overlaps(a_start: usize, a_len: usize, b_start: usize, b_len: usize) -> bool {
    let Some(a_end) = a_start.checked_add(a_len) else {
        return true;
    };
    let Some(b_end) = b_start.checked_add(b_len) else {
        return true;
    };
    a_start < b_end && b_start < a_end
}

fn le_u32(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|candidate| candidate == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::boxed::Box;

    fn image() -> [u8; 512] {
        let mut img = [0u8; 512];
        img[0..8].copy_from_slice(BOOTRT_MAGIC);
        img[8..12].copy_from_slice(&2u32.to_le_bytes());
        img[12..16].copy_from_slice(&128u32.to_le_bytes());
        img[16] = 19;
        img[17] = 3;
        img[18] = BOOTRT_ENTRY.len() as u8;
        img[20..39].copy_from_slice(b"Mirage Boot Runtime");
        img[52..55].copy_from_slice(b"0.1");
        img[64..64 + BOOTRT_ENTRY.len()].copy_from_slice(BOOTRT_ENTRY.as_bytes());
        add_file(&mut img, 0, "/sbin/spider-rs", 384, b"ELF-SPIDER", true);
        add_file(
            &mut img,
            1,
            "/etc/spider/default.target",
            410,
            b"basic.target",
            false,
        );
        img
    }

    fn add_file(
        img: &mut [u8; 512],
        idx: usize,
        path: &str,
        offset: usize,
        data: &[u8],
        entry: bool,
    ) {
        let off = 128 + idx * BOOTRT_ENTRY_SIZE;
        img[offset..offset + data.len()].copy_from_slice(data);
        img[off] = path.len() as u8;
        img[off + 1] = if entry { 1 } else { 0 };
        img[off + 4..off + 8].copy_from_slice(&(offset as u32).to_le_bytes());
        img[off + 8..off + 12].copy_from_slice(&(data.len() as u32).to_le_bytes());
        img[off + 12..off + 16].copy_from_slice(&crc32(data).to_le_bytes());
        img[off + 16..off + 16 + path.len()].copy_from_slice(path.as_bytes());
    }

    #[test]
    fn boot_runtime_manifest_parsing_and_spider_lookup() {
        let img = image();
        let manifest = parse_manifest(&img).unwrap();
        assert_eq!(manifest.file_count, 2);
        assert_eq!(manifest.entry_path(), BOOTRT_ENTRY);
        assert!(find_file_in_manifest(&manifest, BOOTRT_MOUNTED_ENTRY).is_some());
    }

    #[test]
    fn boot_runtime_ramfs_reads_and_rejects_writes() {
        let img = Box::leak(Box::new(image()));
        let (_rt, fs) = BootRuntimeRamFs::mount(&img[..]).unwrap();
        let mut out = [0u8; 10];
        assert_eq!(fs.read(BOOTRT_MOUNTED_ENTRY, 0, &mut out), Ok(10));
        assert_eq!(&out, b"ELF-SPIDER");
        assert_eq!(
            fs.write(BOOTRT_MOUNTED_ENTRY, 0, b"x"),
            Err(BlockError::ReadOnly)
        );
    }
}
