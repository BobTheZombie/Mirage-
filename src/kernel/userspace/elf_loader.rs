//! Minimal ELF64 loader validation for static x86_64 userspace programs.

use super::memory::{UserStack, VirtAddr};
use mirage_mtss::AddressSpaceId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadedProgram {
    pub entry: VirtAddr,
    pub address_space: AddressSpaceId,
    pub user_stack: UserStack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentMapping {
    pub map_start: VirtAddr,
    pub map_len: usize,
    pub page_offset: usize,
    pub file_start: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadSegment {
    pub vaddr: VirtAddr,
    pub file_offset: usize,
    pub file_size: usize,
    pub mem_size: usize,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadError {
    RootFsReadUnavailable,
    TooSmall,
    BadMagic,
    UnsupportedClass,
    UnsupportedEndian,
    UnsupportedType,
    UnsupportedMachine,
    BadProgramHeaderTable,
    BadLoadSegment,
    OverlappingLoadSegments,
    NoLoadSegments,
    EntryNotMapped,
    MapSegmentFailed,
    StackBuildFailed,
}

const EI_CLASS: usize = 4;
const EI_DATA: usize = 5;
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const ELF_HEADER_LEN: usize = 64;
const PHDR_LEN: usize = 56;

pub fn load_elf_from_file(_path: &str) -> Result<LoadedProgram, LoadError> {
    // The root filesystem currently exposes kernel exec scaffolding, but not a
    // stable no-alloc byte read API for this userspace loader. Returning a stub
    // error prevents the boot path from claiming Spider-rs has entered ring 3.
    Err(LoadError::RootFsReadUnavailable)
}

pub fn validate_elf64(image: &[u8]) -> Result<VirtAddr, LoadError> {
    let parsed = parse_elf64(image)?;
    let mut mapped = false;
    let mut idx = 0usize;
    while idx < parsed.segment_count {
        let segment = parsed.segments[idx];
        if (segment.flags & 0x1) != 0 && contains(segment.vaddr.0, segment.mem_size, parsed.entry.0)
        {
            mapped = true;
            break;
        }
        idx += 1;
    }
    if !mapped {
        return Err(LoadError::EntryNotMapped);
    }
    Ok(parsed.entry)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedElf {
    pub entry: VirtAddr,
    pub segments: [LoadSegment; 8],
    pub segment_count: usize,
}

pub fn parse_elf64(image: &[u8]) -> Result<ParsedElf, LoadError> {
    if image.len() < ELF_HEADER_LEN {
        return Err(LoadError::TooSmall);
    }
    if &image[0..4] != b"\x7fELF" {
        return Err(LoadError::BadMagic);
    }
    if image[EI_CLASS] != ELFCLASS64 {
        return Err(LoadError::UnsupportedClass);
    }
    if image[EI_DATA] != ELFDATA2LSB {
        return Err(LoadError::UnsupportedEndian);
    }
    if read_u16(image, 16) != ET_EXEC {
        return Err(LoadError::UnsupportedType);
    }
    if read_u16(image, 18) != EM_X86_64 {
        return Err(LoadError::UnsupportedMachine);
    }

    let entry = VirtAddr(read_u64(image, 24));
    if entry.0 == 0 || entry.0 >= 0x0000_8000_0000_0000 {
        return Err(LoadError::EntryNotMapped);
    }
    let phoff = read_u64(image, 32) as usize;
    let phentsize = read_u16(image, 54) as usize;
    let phnum = read_u16(image, 56) as usize;
    if phnum == 0
        || phentsize != PHDR_LEN
        || phoff
            .checked_add(phentsize.saturating_mul(phnum))
            .map_or(true, |end| end > image.len())
    {
        return Err(LoadError::BadProgramHeaderTable);
    }

    let empty = LoadSegment {
        vaddr: VirtAddr(0),
        file_offset: 0,
        file_size: 0,
        mem_size: 0,
        flags: 0,
    };
    let mut segments = [empty; 8];
    let mut count = 0usize;
    let mut idx = 0usize;
    while idx < phnum {
        let off = phoff + idx * phentsize;
        if read_u32(image, off) == PT_LOAD {
            let flags = read_u32(image, off + 4);
            let p_offset = read_u64(image, off + 8) as usize;
            let p_vaddr = read_u64(image, off + 16);
            let p_filesz = read_u64(image, off + 32) as usize;
            let p_memsz = read_u64(image, off + 40) as usize;
            let p_align = read_u64(image, off + 48);
            if (p_align > 1
                && (!p_align.is_power_of_two()
                    || p_vaddr % p_align != read_u64(image, off + 8) % p_align))
                || p_memsz < p_filesz
                || p_offset
                    .checked_add(p_filesz)
                    .map_or(true, |end| end > image.len())
                || p_vaddr
                    .checked_add(p_memsz as u64)
                    .map_or(true, |end| end > 0x0000_8000_0000_0000)
            {
                return Err(LoadError::BadLoadSegment);
            }
            if count == segments.len() {
                return Err(LoadError::BadProgramHeaderTable);
            }
            segments[count] = LoadSegment {
                vaddr: VirtAddr(p_vaddr),
                file_offset: p_offset,
                file_size: p_filesz,
                mem_size: p_memsz,
                flags,
            };
            count += 1;
        }
        idx += 1;
    }
    if count == 0 {
        return Err(LoadError::NoLoadSegments);
    }
    reject_overlapping_load_segments(&segments, count)?;
    Ok(ParsedElf {
        entry,
        segments,
        segment_count: count,
    })
}

pub const fn segment_mapping(
    vaddr: VirtAddr,
    file_offset: usize,
    mem_size: usize,
) -> SegmentMapping {
    let map_start = vaddr.0 & !0xfff;
    let file_start = file_offset & !0xfff;
    let page_offset = (vaddr.0 - map_start) as usize;
    let map_len = align_up(page_offset + mem_size, 4096);
    SegmentMapping {
        map_start: VirtAddr(map_start),
        map_len,
        page_offset,
        file_start,
    }
}

pub const fn segment_page_bounds(vaddr: VirtAddr, mem_size: usize) -> (VirtAddr, usize) {
    let mapping = segment_mapping(vaddr, 0, mem_size);
    (mapping.map_start, mapping.map_len)
}

fn reject_overlapping_load_segments(
    segments: &[LoadSegment; 8],
    count: usize,
) -> Result<(), LoadError> {
    let mut i = 0usize;
    while i < count {
        let (a_start, a_len) = segment_page_bounds(segments[i].vaddr, segments[i].mem_size);
        let Some(a_end) = a_start.0.checked_add(a_len as u64) else {
            return Err(LoadError::BadLoadSegment);
        };
        let mut j = i + 1;
        while j < count {
            let (b_start, b_len) = segment_page_bounds(segments[j].vaddr, segments[j].mem_size);
            let Some(b_end) = b_start.0.checked_add(b_len as u64) else {
                return Err(LoadError::BadLoadSegment);
            };
            if a_start.0 < b_end && b_start.0 < a_end {
                return Err(LoadError::OverlappingLoadSegments);
            }
            j += 1;
        }
        i += 1;
    }
    Ok(())
}

const fn contains(start: u64, len: usize, value: u64) -> bool {
    match start.checked_add(len as u64) {
        Some(end) => value >= start && value < end,
        None => false,
    }
}

const fn align_up(value: usize, align: usize) -> usize {
    value.saturating_add(align - 1) & !(align - 1)
}

fn read_u16(image: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([image[off], image[off + 1]])
}
fn read_u32(image: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([image[off], image[off + 1], image[off + 2], image[off + 3]])
}
fn read_u64(image: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        image[off],
        image[off + 1],
        image[off + 2],
        image[off + 3],
        image[off + 4],
        image[off + 5],
        image[off + 6],
        image[off + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_elf() -> [u8; 128] {
        let mut image = [0u8; 128];
        image[0..4].copy_from_slice(b"\x7fELF");
        image[EI_CLASS] = ELFCLASS64;
        image[EI_DATA] = ELFDATA2LSB;
        image[16..18].copy_from_slice(&ET_EXEC.to_le_bytes());
        image[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
        image[24..32].copy_from_slice(&0x401000u64.to_le_bytes());
        image[32..40].copy_from_slice(&64u64.to_le_bytes());
        image[54..56].copy_from_slice(&(PHDR_LEN as u16).to_le_bytes());
        image[56..58].copy_from_slice(&1u16.to_le_bytes());
        image[64..68].copy_from_slice(&PT_LOAD.to_le_bytes());
        image[68..72].copy_from_slice(&5u32.to_le_bytes());
        image[72..80].copy_from_slice(&0u64.to_le_bytes());
        image[80..88].copy_from_slice(&0x400000u64.to_le_bytes());
        image[96..104].copy_from_slice(&16u64.to_le_bytes());
        image[104..112].copy_from_slice(&0x2000u64.to_le_bytes());
        image
    }

    #[test]
    fn elf_header_validation_accepts_static_x86_64_exec() {
        assert_eq!(validate_elf64(&minimal_elf()), Ok(VirtAddr(0x401000)));
    }

    #[test]
    fn elf_header_validation_rejects_bad_magic() {
        assert_eq!(validate_elf64(&[0; 128]), Err(LoadError::BadMagic));
    }

    #[test]
    fn parser_records_load_segment_file_offset() {
        let parsed = parse_elf64(&minimal_elf()).unwrap();
        assert_eq!(parsed.segment_count, 1);
        assert_eq!(parsed.segments[0].file_offset, 0);
        assert_eq!(parsed.segments[0].mem_size, 0x2000);
    }

    #[test]
    fn parser_rejects_overlapping_load_segments() {
        let mut image = [0u8; 184];
        image[..128].copy_from_slice(&minimal_elf());
        image[56..58].copy_from_slice(&2u16.to_le_bytes());
        image[120..124].copy_from_slice(&PT_LOAD.to_le_bytes());
        image[124..128].copy_from_slice(&6u32.to_le_bytes());
        image[128..136].copy_from_slice(&0u64.to_le_bytes());
        image[136..144].copy_from_slice(&0x401000u64.to_le_bytes());
        image[152..160].copy_from_slice(&8u64.to_le_bytes());
        image[160..168].copy_from_slice(&0x1000u64.to_le_bytes());
        assert_eq!(parse_elf64(&image), Err(LoadError::OverlappingLoadSegments));
    }

    #[test]
    fn parser_rejects_elf_without_load_segments() {
        let mut image = minimal_elf();
        image[64..68].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(parse_elf64(&image), Err(LoadError::NoLoadSegments));
    }

    #[test]
    fn pt_load_mapping_math_includes_unaligned_prefix() {
        assert_eq!(
            segment_page_bounds(VirtAddr(0x401123), 0x2000),
            (VirtAddr(0x401000), 0x3000)
        );
    }
}
