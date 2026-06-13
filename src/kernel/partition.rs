//! MBR/GPT partition parsing above the generic block layer.

use crate::kernel::block::{BlockDevice, BlockError};

pub const MAX_PARTITIONS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartitionTableKind {
    Mbr,
    Gpt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartitionInfo {
    pub index: u8,
    pub first_lba: u64,
    pub block_count: u64,
    pub type_code: u8,
    pub bootable: bool,
    pub name: [u8; 36],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartitionTable {
    pub kind: PartitionTableKind,
    pub partitions: [Option<PartitionInfo>; MAX_PARTITIONS],
    pub count: usize,
    pub protective_mbr: bool,
}

impl PartitionTable {
    const fn empty(kind: PartitionTableKind, protective_mbr: bool) -> Self {
        Self {
            kind,
            partitions: [None; MAX_PARTITIONS],
            count: 0,
            protective_mbr,
        }
    }

    fn push(&mut self, partition: PartitionInfo) {
        if self.count < MAX_PARTITIONS {
            self.partitions[self.count] = Some(partition);
            self.count += 1;
        }
    }
}

pub fn parse_partitions(device: &dyn BlockDevice) -> Result<PartitionTable, BlockError> {
    match parse_gpt(device) {
        Ok(table) => Ok(table),
        Err(_) => parse_mbr(device),
    }
}

pub fn parse_mbr(device: &dyn BlockDevice) -> Result<PartitionTable, BlockError> {
    let info = device.info();
    let mut sector = [0u8; 512];
    device.read_blocks(0, 1, &mut sector)?;
    parse_mbr_sector(&sector, info.block_count)
}

pub fn parse_mbr_sector(
    sector: &[u8; 512],
    device_blocks: u64,
) -> Result<PartitionTable, BlockError> {
    if sector[510] != 0x55 || sector[511] != 0xaa {
        return Err(BlockError::InvalidSignature);
    }
    let mut table = PartitionTable::empty(PartitionTableKind::Mbr, false);
    for index in 0..4 {
        let off = 446 + index * 16;
        let type_code = sector[off + 4];
        let first_lba = u32::from_le_bytes([
            sector[off + 8],
            sector[off + 9],
            sector[off + 10],
            sector[off + 11],
        ]) as u64;
        let block_count = u32::from_le_bytes([
            sector[off + 12],
            sector[off + 13],
            sector[off + 14],
            sector[off + 15],
        ]) as u64;
        if type_code == 0 || block_count == 0 {
            continue;
        }
        if type_code == 0xee {
            table.protective_mbr = true;
        }
        if first_lba
            .checked_add(block_count)
            .map_or(true, |end| end > device_blocks)
        {
            return Err(BlockError::OutOfBounds);
        }
        table.push(PartitionInfo {
            index: index as u8 + 1,
            first_lba,
            block_count,
            type_code,
            bootable: sector[off] == 0x80,
            name: [0; 36],
        });
    }
    Ok(table)
}

pub fn parse_gpt(device: &dyn BlockDevice) -> Result<PartitionTable, BlockError> {
    let info = device.info();
    if info.block_count < 2 {
        return Err(BlockError::OutOfBounds);
    }
    let mut lba0 = [0u8; 512];
    device.read_blocks(0, 1, &mut lba0)?;
    let mbr = parse_mbr_sector(&lba0, info.block_count)?;
    if !mbr.protective_mbr {
        return Err(BlockError::InvalidSignature);
    }
    let mut header = [0u8; 512];
    device.read_blocks(1, 1, &mut header)?;
    parse_gpt_header_and_entries(device, &header)
}

pub fn parse_gpt_header_and_entries(
    device: &dyn BlockDevice,
    header: &[u8; 512],
) -> Result<PartitionTable, BlockError> {
    if &header[0..8] != b"EFI PART" {
        return Err(BlockError::InvalidSignature);
    }
    let header_size = le_u32(header, 12) as usize;
    if !(92..=512).contains(&header_size) {
        return Err(BlockError::InvalidSignature);
    }
    let stored_crc = le_u32(header, 16);
    let mut crc_header = [0u8; 512];
    crc_header.copy_from_slice(header);
    crc_header[16..20].copy_from_slice(&0u32.to_le_bytes());
    if crc32(&crc_header[..header_size]) != stored_crc {
        return Err(BlockError::Crc);
    }
    let first_usable = le_u64(header, 40);
    let last_usable = le_u64(header, 48);
    let entries_lba = le_u64(header, 72);
    let entry_count = le_u32(header, 80) as usize;
    let entry_size = le_u32(header, 84) as usize;
    let entries_crc = le_u32(header, 88);
    if entry_size < 128 || entry_size > 512 || entry_count == 0 {
        return Err(BlockError::InvalidSignature);
    }
    let total_bytes = entry_count
        .checked_mul(entry_size)
        .ok_or(BlockError::OutOfBounds)?;
    let sectors = (total_bytes + 511) / 512;
    if sectors > 32 {
        return Err(BlockError::Unsupported);
    }
    let mut entries = [0u8; 32 * 512];
    device.read_blocks(entries_lba, sectors as u32, &mut entries[..sectors * 512])?;
    if crc32(&entries[..total_bytes]) != entries_crc {
        return Err(BlockError::Crc);
    }
    let mut table = PartitionTable::empty(PartitionTableKind::Gpt, true);
    let limit = core::cmp::min(entry_count, MAX_PARTITIONS);
    for index in 0..limit {
        let off = index * entry_size;
        if entries[off..off + 16].iter().all(|b| *b == 0) {
            continue;
        }
        let first_lba = le_u64(&entries, off + 32);
        let last_lba = le_u64(&entries, off + 40);
        if first_lba < first_usable || last_lba > last_usable || last_lba < first_lba {
            return Err(BlockError::OutOfBounds);
        }
        let mut name = [0u8; 36];
        utf16le_name_to_ascii(&entries[off + 56..off + 128], &mut name);
        table.push(PartitionInfo {
            index: index as u8 + 1,
            first_lba,
            block_count: last_lba - first_lba + 1,
            type_code: 0xee,
            bootable: false,
            name,
        });
    }
    Ok(table)
}

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn utf16le_name_to_ascii(input: &[u8], out: &mut [u8]) {
    let mut i = 0usize;
    let mut j = 0usize;
    while i + 1 < input.len() && j < out.len() {
        let code = u16::from_le_bytes([input[i], input[i + 1]]);
        if code == 0 {
            break;
        }
        out[j] = if code <= 0x7f { code as u8 } else { b'?' };
        i += 2;
        j += 1;
    }
}

fn le_u32(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

fn le_u64(bytes: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        bytes[off],
        bytes[off + 1],
        bytes[off + 2],
        bytes[off + 3],
        bytes[off + 4],
        bytes[off + 5],
        bytes[off + 6],
        bytes[off + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::block::{BlockDeviceInfo, BlockDeviceKind};

    struct Disk {
        data: [u8; 34 * 512],
    }

    impl BlockDevice for Disk {
        fn info(&self) -> BlockDeviceInfo {
            BlockDeviceInfo {
                id: 1,
                name: "disk0",
                kind: BlockDeviceKind::RamDisk,
                block_size: 512,
                block_count: 34,
                readonly: true,
            }
        }
        fn read_blocks(&self, lba: u64, count: u32, buffer: &mut [u8]) -> Result<(), BlockError> {
            let start = lba as usize * 512;
            let len = count as usize * 512;
            buffer[..len].copy_from_slice(&self.data[start..start + len]);
            Ok(())
        }
        fn write_blocks(&self, _lba: u64, _count: u32, _buffer: &[u8]) -> Result<(), BlockError> {
            Err(BlockError::ReadOnly)
        }
        fn flush(&self) -> Result<(), BlockError> {
            Ok(())
        }
    }

    #[test]
    fn mbr_parser_accepts_primary_and_protective_entries() {
        let mut sector = [0u8; 512];
        sector[446] = 0x80;
        sector[450] = 0x83;
        sector[454..458].copy_from_slice(&1u32.to_le_bytes());
        sector[458..462].copy_from_slice(&8u32.to_le_bytes());
        sector[462 + 4] = 0xee;
        sector[462 + 8..462 + 12].copy_from_slice(&1u32.to_le_bytes());
        sector[462 + 12..462 + 16].copy_from_slice(&33u32.to_le_bytes());
        sector[510] = 0x55;
        sector[511] = 0xaa;
        let table = parse_mbr_sector(&sector, 40).unwrap();
        assert_eq!(table.kind, PartitionTableKind::Mbr);
        assert_eq!(table.count, 2);
        assert!(table.protective_mbr);
        assert_eq!(table.partitions[0].unwrap().first_lba, 1);
    }

    #[test]
    fn gpt_crc_and_partition_entry_parsing() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        let mut disk = Disk {
            data: [0; 34 * 512],
        };
        disk.data[510] = 0x55;
        disk.data[511] = 0xaa;
        disk.data[450] = 0xee;
        disk.data[454..458].copy_from_slice(&1u32.to_le_bytes());
        disk.data[458..462].copy_from_slice(&33u32.to_le_bytes());

        let h = 512;
        disk.data[h..h + 8].copy_from_slice(b"EFI PART");
        disk.data[h + 8..h + 12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        disk.data[h + 12..h + 16].copy_from_slice(&92u32.to_le_bytes());
        disk.data[h + 40..h + 48].copy_from_slice(&2u64.to_le_bytes());
        disk.data[h + 48..h + 56].copy_from_slice(&32u64.to_le_bytes());
        disk.data[h + 72..h + 80].copy_from_slice(&2u64.to_le_bytes());
        disk.data[h + 80..h + 84].copy_from_slice(&1u32.to_le_bytes());
        disk.data[h + 84..h + 88].copy_from_slice(&128u32.to_le_bytes());

        let e = 2 * 512;
        disk.data[e] = 1;
        disk.data[e + 32..e + 40].copy_from_slice(&3u64.to_le_bytes());
        disk.data[e + 40..e + 48].copy_from_slice(&8u64.to_le_bytes());
        disk.data[e + 56..e + 58].copy_from_slice(&(b'r' as u16).to_le_bytes());
        disk.data[e + 58..e + 60].copy_from_slice(&(b'o' as u16).to_le_bytes());
        let entries_crc = crc32(&disk.data[e..e + 128]);
        disk.data[h + 88..h + 92].copy_from_slice(&entries_crc.to_le_bytes());
        let mut header = [0u8; 512];
        header.copy_from_slice(&disk.data[h..h + 512]);
        let header_crc = crc32(&header[..92]);
        header[16..20].copy_from_slice(&header_crc.to_le_bytes());
        disk.data[h..h + 512].copy_from_slice(&header);

        let table = parse_gpt(&disk).unwrap();
        assert_eq!(table.kind, PartitionTableKind::Gpt);
        assert_eq!(table.count, 1);
        assert_eq!(table.partitions[0].unwrap().block_count, 6);
        assert_eq!(&table.partitions[0].unwrap().name[..2], b"ro");
    }
}
