//! In-kernel ELF64 executable loader for `execve`.
//!
//! This module keeps the public syscall number/argument ABI stable while moving
//! the authority to choose the new instruction and stack pointers into the
//! kernel.  Userspace now supplies only the path, vectors, and credential hints;
//! the loader validates the executable image and constructs the initial process
//! stack itself.

use crate::kernel::fs::{AccessMode, FileSystem, InodeKind, OpenFlags, VfsError};
use crate::kernel::memory::{self, MemoryProtection, PAGE_SIZE};
use crate::kernel::process::{
    ExecImageMetadata, ExecServiceDaemon, ExecSignatureMetadata, ExecVectorMetadata, ProcessId,
};
use crate::kernel::{Kernel, KernelError, KernelPathBuf, KernelResult};

const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const MAX_PROGRAM_HEADERS: usize = 64;
const PT_LOAD: u32 = 1;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const EM_X86_64: u16 = 62;
const PF_X: u32 = 0x1;
const PF_W: u32 = 0x2;
const PF_R: u32 = 0x4;
const PIE_LOAD_BASE: u64 = 0x0000_5555_5555_0000;
const USER_STACK_BASE: u64 = 0x0000_7fff_fff0_0000;
const USER_STACK_SIZE: usize = 64 * 1024;
const USER_CANONICAL_LIMIT: u64 = 0x0000_8000_0000_0000;

#[derive(Clone, Copy, Debug)]
struct ElfHeader {
    file_type: u16,
    machine: u16,
    entry: u64,
    phoff: u64,
    phentsize: u16,
    phnum: u16,
}

#[derive(Clone, Copy, Debug)]
struct ProgramHeader {
    kind: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
}

impl<const MAX_PROC: usize, const MSG_DEPTH: usize> Kernel<MAX_PROC, MSG_DEPTH> {
    pub(super) fn load_exec_image(
        &mut self,
        caller: ProcessId,
        resolved: &KernelPathBuf,
        stat: crate::kernel::fs::inode::Stat,
        argv: ExecVectorMetadata,
        envp: ExecVectorMetadata,
    ) -> KernelResult<ExecImageMetadata> {
        if stat.kind != InodeKind::RegularFile {
            return Err(KernelError::Filesystem(VfsError::PermissionDenied));
        }
        if !crate::kernel::fs::Permissions::new(stat.mode, stat.uid, stat.gid)
            .allows(self.fs_credentials_for(caller)?, AccessMode::Execute)
        {
            return Err(KernelError::Filesystem(VfsError::PermissionDenied));
        }

        let path = resolved.as_path()?;
        let file = self
            .root_fs
            .open(path, OpenFlags::RDONLY, self.fs_credentials_for(caller)?)
            .map_err(KernelError::Filesystem)?;
        let header = self.read_elf_header(&file, stat.size)?;
        let address_space_root =
            memory::create_user_address_space(caller).ok_or(KernelError::AllocationFailed)?;
        let entry_point = relocated_address(header.entry, header.file_type)?;
        if let Err(error) = self.load_elf_segments(
            caller,
            address_space_root,
            &file,
            stat.size,
            header,
            entry_point,
        ) {
            let _ = self.root_fs.close(file);
            memory::destroy_user_address_space(address_space_root);
            return Err(error);
        }
        let stack_pointer = match self.build_initial_stack(caller, address_space_root, argv, envp) {
            Ok(stack_pointer) => stack_pointer,
            Err(error) => {
                let _ = self.root_fs.close(file);
                memory::destroy_user_address_space(address_space_root);
                return Err(error);
            }
        };
        self.root_fs.close(file).map_err(KernelError::Filesystem)?;

        let (service_daemon, signature) = signed_exec_manifest_for_path(resolved.as_str());
        Ok(ExecImageMetadata::new(
            stat.inode.raw(),
            stat.size,
            stat.mode,
            entry_point,
            stack_pointer,
            address_space_root,
            service_daemon,
            signature,
        ))
    }

    fn read_elf_header(
        &self,
        file: &crate::kernel::fs::File,
        file_size: u64,
    ) -> KernelResult<ElfHeader> {
        if file_size < ELF_HEADER_SIZE as u64 {
            return Err(KernelError::InvalidArgument);
        }
        let mut bytes = [0u8; ELF_HEADER_SIZE];
        read_exact(&self.root_fs, file, &mut bytes, 0)?;
        if &bytes[0..4] != b"\x7fELF" {
            return Err(KernelError::InvalidArgument);
        }
        if bytes[4] != 2 || bytes[5] != 1 || bytes[6] != 1 {
            return Err(KernelError::InvalidArgument);
        }
        let header = ElfHeader {
            file_type: u16_at(&bytes, 16),
            machine: u16_at(&bytes, 18),
            entry: u64_at(&bytes, 24),
            phoff: u64_at(&bytes, 32),
            phentsize: u16_at(&bytes, 54),
            phnum: u16_at(&bytes, 56),
        };
        if !matches!(header.file_type, ET_EXEC | ET_DYN)
            || header.machine != EM_X86_64
            || header.phentsize as usize != PROGRAM_HEADER_SIZE
            || header.phnum == 0
            || header.phnum as usize > MAX_PROGRAM_HEADERS
        {
            return Err(KernelError::InvalidArgument);
        }
        let ph_bytes = (header.phnum as u64)
            .checked_mul(header.phentsize as u64)
            .ok_or(KernelError::InvalidArgument)?;
        let ph_end = header
            .phoff
            .checked_add(ph_bytes)
            .ok_or(KernelError::InvalidArgument)?;
        if ph_end > file_size {
            return Err(KernelError::InvalidArgument);
        }
        Ok(header)
    }

    fn load_elf_segments(
        &self,
        owner: ProcessId,
        address_space_root: u64,
        file: &crate::kernel::fs::File,
        file_size: u64,
        header: ElfHeader,
        entry_point: u64,
    ) -> KernelResult<()> {
        let mut index = 0u16;
        let mut saw_load = false;
        let mut entry_mapped_executable = false;
        while index < header.phnum {
            let ph = self.read_program_header(file, header.phoff, index)?;
            if ph.kind == PT_LOAD {
                saw_load = true;
                validate_load_header(ph, file_size)?;
                let relocated = relocated_address(ph.vaddr, header.file_type)?;
                let map_start = align_down(relocated);
                let page_offset = (relocated - map_start) as usize;
                let map_len = align_up_usize(
                    page_offset
                        .checked_add(ph.memsz as usize)
                        .ok_or(KernelError::InvalidArgument)?,
                )?;
                let segment_start = relocated;
                let segment_end = relocated
                    .checked_add(ph.memsz)
                    .ok_or(KernelError::InvalidArgument)?;
                let protection = protection_from_elf_flags(ph.flags);
                if (ph.flags & PF_X) != 0
                    && entry_point >= segment_start
                    && entry_point < segment_end
                {
                    entry_mapped_executable = true;
                }
                let region = memory::mmap_user_fixed(
                    owner,
                    address_space_root,
                    map_start,
                    map_len,
                    protection,
                )
                .ok_or(KernelError::AllocationFailed)?;
                unsafe {
                    core::ptr::write_bytes(region.as_ptr(), 0, region.length);
                }
                if ph.filesz != 0 {
                    let dest = unsafe {
                        core::slice::from_raw_parts_mut(
                            region.as_ptr().add(page_offset),
                            ph.filesz as usize,
                        )
                    };
                    read_exact(&self.root_fs, file, dest, ph.offset)?;
                }
            }
            index += 1;
        }
        if saw_load && entry_mapped_executable {
            Ok(())
        } else {
            Err(KernelError::InvalidArgument)
        }
    }

    fn read_program_header(
        &self,
        file: &crate::kernel::fs::File,
        phoff: u64,
        index: u16,
    ) -> KernelResult<ProgramHeader> {
        let mut bytes = [0u8; PROGRAM_HEADER_SIZE];
        let offset = phoff
            .checked_add((index as u64) * PROGRAM_HEADER_SIZE as u64)
            .ok_or(KernelError::InvalidArgument)?;
        read_exact(&self.root_fs, file, &mut bytes, offset)?;
        Ok(ProgramHeader {
            kind: u32_at(&bytes, 0),
            flags: u32_at(&bytes, 4),
            offset: u64_at(&bytes, 8),
            vaddr: u64_at(&bytes, 16),
            filesz: u64_at(&bytes, 32),
            memsz: u64_at(&bytes, 40),
            align: u64_at(&bytes, 48),
        })
    }

    fn build_initial_stack(
        &self,
        owner: ProcessId,
        address_space_root: u64,
        argv: ExecVectorMetadata,
        envp: ExecVectorMetadata,
    ) -> KernelResult<u64> {
        let stack = memory::mmap_user_fixed(
            owner,
            address_space_root,
            USER_STACK_BASE,
            USER_STACK_SIZE,
            MemoryProtection::read_write(),
        )
        .ok_or(KernelError::AllocationFailed)?;
        unsafe {
            core::ptr::write_bytes(stack.as_ptr(), 0, stack.length);
        }
        let mut builder = InitialStackBuilder::new(stack.as_ptr(), USER_STACK_BASE, stack.length);
        let argv_ptrs = builder.copy_vector_strings(argv)?;
        let envp_ptrs = builder.copy_vector_strings(envp)?;
        builder.finish(argv_ptrs, argv.count, envp_ptrs, envp.count)
    }
}

struct InitialStackBuilder {
    base: *mut u8,
    user_base: u64,
    len: usize,
    sp: u64,
}

impl InitialStackBuilder {
    fn new(base: *mut u8, user_base: u64, len: usize) -> Self {
        Self {
            base,
            user_base,
            len,
            sp: user_base + len as u64,
        }
    }

    fn copy_vector_strings(
        &mut self,
        vector: ExecVectorMetadata,
    ) -> KernelResult<[u64; crate::kernel::process::MAX_EXEC_ARGS]> {
        let mut copied = [0u64; crate::kernel::process::MAX_EXEC_ARGS];
        let mut idx = 0usize;
        while idx < vector.count {
            let string_ptr = super::read_user_value::<u64>(vector.base + (idx * 8) as u64)?;
            let bytes = super::user_cstr(string_ptr)?;
            let total = bytes
                .len()
                .checked_add(1)
                .ok_or(KernelError::InvalidArgument)?;
            self.sp = self
                .sp
                .checked_sub(total as u64)
                .ok_or(KernelError::InvalidArgument)?;
            self.ensure_in_stack(self.sp, total)?;
            let offset = (self.sp - self.user_base) as usize;
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.base.add(offset), bytes.len());
                *self.base.add(offset + bytes.len()) = 0;
            }
            copied[idx] = self.sp;
            idx += 1;
        }
        Ok(copied)
    }

    fn finish(
        &mut self,
        argv_ptrs: [u64; crate::kernel::process::MAX_EXEC_ARGS],
        argc: usize,
        envp_ptrs: [u64; crate::kernel::process::MAX_EXEC_ARGS],
        envc: usize,
    ) -> KernelResult<u64> {
        self.sp &= !0xf;
        self.push_u64(0)?;
        self.push_u64(0)?;
        self.push_u64(0)?;
        let mut idx = envc;
        while idx > 0 {
            idx -= 1;
            self.push_u64(envp_ptrs[idx])?;
        }
        self.push_u64(0)?;
        idx = argc;
        while idx > 0 {
            idx -= 1;
            self.push_u64(argv_ptrs[idx])?;
        }
        self.push_u64(argc as u64)?;
        Ok(self.sp)
    }

    fn push_u64(&mut self, value: u64) -> KernelResult<()> {
        self.sp = self.sp.checked_sub(8).ok_or(KernelError::InvalidArgument)?;
        self.ensure_in_stack(self.sp, 8)?;
        let offset = (self.sp - self.user_base) as usize;
        unsafe {
            core::ptr::copy_nonoverlapping(
                value.to_le_bytes().as_ptr(),
                self.base.add(offset),
                core::mem::size_of::<u64>(),
            );
        }
        Ok(())
    }

    fn ensure_in_stack(&self, user_address: u64, len: usize) -> KernelResult<()> {
        if user_address < self.user_base
            || user_address
                .checked_add(len as u64)
                .filter(|end| *end <= self.user_base + self.len as u64)
                .is_none()
        {
            return Err(KernelError::InvalidArgument);
        }
        Ok(())
    }
}

fn read_exact<F: FileSystem + ?Sized>(
    fs: &F,
    file: &crate::kernel::fs::File,
    buffer: &mut [u8],
    offset: u64,
) -> KernelResult<()> {
    let read = fs
        .pread(file, buffer, offset)
        .map_err(KernelError::Filesystem)?;
    if read == buffer.len() {
        Ok(())
    } else {
        Err(KernelError::Filesystem(VfsError::InvalidHandle))
    }
}

fn validate_load_header(ph: ProgramHeader, file_size: u64) -> KernelResult<()> {
    if ph.memsz < ph.filesz || ph.align == 0 || !ph.align.is_power_of_two() {
        return Err(KernelError::InvalidArgument);
    }
    if ph.vaddr % ph.align != ph.offset % ph.align {
        return Err(KernelError::InvalidArgument);
    }
    ph.offset
        .checked_add(ph.filesz)
        .filter(|end| *end <= file_size)
        .ok_or(KernelError::InvalidArgument)?;
    ph.vaddr
        .checked_add(ph.memsz)
        .filter(|end| *end < USER_CANONICAL_LIMIT)
        .ok_or(KernelError::InvalidArgument)?;
    Ok(())
}

fn relocated_address(address: u64, file_type: u16) -> KernelResult<u64> {
    match file_type {
        ET_EXEC => Ok(address),
        ET_DYN => PIE_LOAD_BASE
            .checked_add(address)
            .filter(|value| *value < USER_CANONICAL_LIMIT)
            .ok_or(KernelError::InvalidArgument),
        _ => Err(KernelError::InvalidArgument),
    }
}

fn protection_from_elf_flags(flags: u32) -> MemoryProtection {
    MemoryProtection::new(
        (flags & PF_R) != 0,
        (flags & PF_W) != 0,
        (flags & PF_X) != 0,
    )
}

fn align_down(value: u64) -> u64 {
    value & !((PAGE_SIZE as u64) - 1)
}

fn align_up_usize(value: usize) -> KernelResult<usize> {
    value
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value & !(PAGE_SIZE - 1))
        .ok_or(KernelError::InvalidArgument)
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn signed_exec_manifest_for_path(
    path: &str,
) -> (Option<ExecServiceDaemon>, Option<ExecSignatureMetadata>) {
    super::signed_exec_manifest_for_path(path)
}
