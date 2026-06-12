#![no_std]
#![forbid(unsafe_code)]

//! Bounds-checked ACPI table parsers for early Mirage hardware discovery.
//!
//! The crate parses firmware-provided bytes only. It does not dereference
//! physical addresses, execute AML, enable EC regions, or make power policy.

extern crate alloc;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpiError {
    Truncated,
    BadSignature,
    BadChecksum,
    UnsupportedRevision,
    InvalidLength,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rsdp {
    pub revision: u8,
    pub rsdt_address: u32,
    pub xsdt_address: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdtHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FadtInfo {
    pub firmware_ctrl: u32,
    pub dsdt: u32,
    pub preferred_pm_profile: u8,
    pub sci_interrupt: u16,
    pub smi_command_port: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MadtInfo {
    pub lapic_address: u32,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HpetInfo {
    pub event_timer_block_id: u32,
    pub base_address: u64,
    pub sequence: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McfgAllocation {
    pub base_address: u64,
    pub pci_segment_group: u16,
    pub start_bus: u8,
    pub end_bus: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IvrsHeader {
    pub iv_info: u32,
    pub entries_offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcpiTablePresence {
    pub fadt: bool,
    pub madt: bool,
    pub hpet: bool,
    pub mcfg: bool,
    pub ivrs: bool,
}

pub fn parse_rsdp(bytes: &[u8]) -> Result<Rsdp, AcpiError> {
    if bytes.len() < 20 {
        return Err(AcpiError::Truncated);
    }
    if &bytes[0..8] != b"RSD PTR " {
        return Err(AcpiError::BadSignature);
    }
    if checksum(&bytes[..20]) != 0 {
        return Err(AcpiError::BadChecksum);
    }
    let revision = bytes[15];
    let rsdt_address = le_u32(bytes, 16)?;
    let xsdt_address = if revision >= 2 {
        if bytes.len() < 36 {
            return Err(AcpiError::Truncated);
        }
        let length = le_u32(bytes, 20)? as usize;
        if length < 36 || bytes.len() < length {
            return Err(AcpiError::InvalidLength);
        }
        if checksum(&bytes[..length]) != 0 {
            return Err(AcpiError::BadChecksum);
        }
        Some(le_u64(bytes, 24)?)
    } else {
        None
    };
    Ok(Rsdp {
        revision,
        rsdt_address,
        xsdt_address,
    })
}

pub fn parse_sdt_header(bytes: &[u8]) -> Result<SdtHeader, AcpiError> {
    if bytes.len() < 36 {
        return Err(AcpiError::Truncated);
    }
    let length = le_u32(bytes, 4)?;
    if length < 36 || bytes.len() < length as usize {
        return Err(AcpiError::InvalidLength);
    }
    if checksum(&bytes[..length as usize]) != 0 {
        return Err(AcpiError::BadChecksum);
    }
    let mut signature = [0u8; 4];
    signature.copy_from_slice(&bytes[0..4]);
    let mut oem_id = [0u8; 6];
    oem_id.copy_from_slice(&bytes[10..16]);
    let mut oem_table_id = [0u8; 8];
    oem_table_id.copy_from_slice(&bytes[16..24]);
    Ok(SdtHeader {
        signature,
        length,
        revision: bytes[8],
        oem_id,
        oem_table_id,
    })
}

pub fn parse_rsdt_entries(bytes: &[u8]) -> Result<Vec<u32>, AcpiError> {
    let header = parse_sdt_header(bytes)?;
    if &header.signature != b"RSDT" {
        return Err(AcpiError::BadSignature);
    }
    let payload = (header.length as usize)
        .checked_sub(36)
        .ok_or(AcpiError::InvalidLength)?;
    if payload % 4 != 0 {
        return Err(AcpiError::InvalidLength);
    }
    let mut entries = Vec::new();
    let mut offset = 36usize;
    while offset < header.length as usize {
        entries.push(le_u32(bytes, offset)?);
        offset += 4;
    }
    Ok(entries)
}

pub fn parse_xsdt_entries(bytes: &[u8]) -> Result<Vec<u64>, AcpiError> {
    let header = parse_sdt_header(bytes)?;
    if &header.signature != b"XSDT" {
        return Err(AcpiError::BadSignature);
    }
    let payload = (header.length as usize)
        .checked_sub(36)
        .ok_or(AcpiError::InvalidLength)?;
    if payload % 8 != 0 {
        return Err(AcpiError::InvalidLength);
    }
    let mut entries = Vec::new();
    let mut offset = 36usize;
    while offset < header.length as usize {
        entries.push(le_u64(bytes, offset)?);
        offset += 8;
    }
    Ok(entries)
}

pub fn parse_fadt(bytes: &[u8]) -> Result<FadtInfo, AcpiError> {
    let header = parse_sdt_header(bytes)?;
    if &header.signature != b"FACP" {
        return Err(AcpiError::BadSignature);
    }
    if header.length < 52 {
        return Err(AcpiError::InvalidLength);
    }
    Ok(FadtInfo {
        firmware_ctrl: le_u32(bytes, 36)?,
        dsdt: le_u32(bytes, 40)?,
        preferred_pm_profile: bytes[45],
        sci_interrupt: le_u16(bytes, 46)?,
        smi_command_port: le_u32(bytes, 48)?,
    })
}

pub fn parse_madt(bytes: &[u8]) -> Result<MadtInfo, AcpiError> {
    let header = parse_sdt_header(bytes)?;
    if &header.signature != b"APIC" {
        return Err(AcpiError::BadSignature);
    }
    if header.length < 44 {
        return Err(AcpiError::InvalidLength);
    }
    Ok(MadtInfo {
        lapic_address: le_u32(bytes, 36)?,
        flags: le_u32(bytes, 40)?,
    })
}

pub fn parse_hpet(bytes: &[u8]) -> Result<HpetInfo, AcpiError> {
    let header = parse_sdt_header(bytes)?;
    if &header.signature != b"HPET" {
        return Err(AcpiError::BadSignature);
    }
    if header.length < 56 {
        return Err(AcpiError::InvalidLength);
    }
    Ok(HpetInfo {
        event_timer_block_id: le_u32(bytes, 36)?,
        base_address: le_u64(bytes, 44)?,
        sequence: bytes[52],
    })
}

pub fn parse_mcfg(bytes: &[u8]) -> Result<Vec<McfgAllocation>, AcpiError> {
    let header = parse_sdt_header(bytes)?;
    if &header.signature != b"MCFG" {
        return Err(AcpiError::BadSignature);
    }
    let payload = (header.length as usize)
        .checked_sub(44)
        .ok_or(AcpiError::InvalidLength)?;
    if payload % 16 != 0 {
        return Err(AcpiError::InvalidLength);
    }
    let mut allocations = Vec::new();
    let mut offset = 44usize;
    while offset < header.length as usize {
        allocations.push(McfgAllocation {
            base_address: le_u64(bytes, offset)?,
            pci_segment_group: le_u16(bytes, offset + 8)?,
            start_bus: bytes[offset + 10],
            end_bus: bytes[offset + 11],
        });
        offset += 16;
    }
    Ok(allocations)
}

pub fn parse_ivrs_header(bytes: &[u8]) -> Result<IvrsHeader, AcpiError> {
    let header = parse_sdt_header(bytes)?;
    if &header.signature != b"IVRS" {
        return Err(AcpiError::BadSignature);
    }
    if header.length < 48 {
        return Err(AcpiError::InvalidLength);
    }
    Ok(IvrsHeader {
        iv_info: le_u32(bytes, 36)?,
        entries_offset: 48,
    })
}

pub fn summarize_tables(tables: &[&[u8]]) -> Result<AcpiTablePresence, AcpiError> {
    let mut presence = AcpiTablePresence {
        fadt: false,
        madt: false,
        hpet: false,
        mcfg: false,
        ivrs: false,
    };
    for table in tables {
        let header = parse_sdt_header(table)?;
        match &header.signature {
            b"FACP" => presence.fadt = true,
            b"APIC" => presence.madt = true,
            b"HPET" => presence.hpet = true,
            b"MCFG" => presence.mcfg = true,
            b"IVRS" => presence.ivrs = true,
            _ => {}
        }
    }
    Ok(presence)
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}
fn le_u16(bytes: &[u8], offset: usize) -> Result<u16, AcpiError> {
    Ok(u16::from_le_bytes(read(bytes, offset)?))
}
fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, AcpiError> {
    Ok(u32::from_le_bytes(read(bytes, offset)?))
}
fn le_u64(bytes: &[u8], offset: usize) -> Result<u64, AcpiError> {
    Ok(u64::from_le_bytes(read(bytes, offset)?))
}
fn read<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], AcpiError> {
    let end = offset.checked_add(N).ok_or(AcpiError::InvalidLength)?;
    let slice = bytes.get(offset..end).ok_or(AcpiError::Truncated)?;
    let mut out = [0u8; N];
    out.copy_from_slice(slice);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn table(sig: &[u8; 4], len: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; len];
        bytes[0..4].copy_from_slice(sig);
        bytes[4..8].copy_from_slice(&(len as u32).to_le_bytes());
        bytes[8] = 1;
        let sum = bytes.iter().fold(0u8, |s, b| s.wrapping_add(*b));
        bytes[9] = (0u8).wrapping_sub(sum);
        bytes
    }

    #[test]
    fn parses_xsdt_entries_with_checksum() {
        let mut xsdt = table(b"XSDT", 52);
        xsdt[36..44].copy_from_slice(&0x1234_5000u64.to_le_bytes());
        xsdt[44..52].copy_from_slice(&0x5678_9000u64.to_le_bytes());
        xsdt[9] = 0;
        let sum = xsdt.iter().fold(0u8, |s, b| s.wrapping_add(*b));
        xsdt[9] = (0u8).wrapping_sub(sum);
        assert_eq!(
            parse_xsdt_entries(&xsdt).unwrap(),
            vec![0x1234_5000, 0x5678_9000]
        );
    }

    #[test]
    fn parses_required_ryzen_tables() {
        let mut fadt = table(b"FACP", 52);
        fadt[46..48].copy_from_slice(&9u16.to_le_bytes());
        fadt[9] = 0;
        let sum = fadt.iter().fold(0u8, |s, b| s.wrapping_add(*b));
        fadt[9] = (0u8).wrapping_sub(sum);
        let mcfg = table(b"MCFG", 60);
        let ivrs = table(b"IVRS", 48);
        let presence = summarize_tables(&[&fadt, &mcfg, &ivrs]).unwrap();
        assert!(presence.fadt && presence.mcfg && presence.ivrs);
        assert_eq!(parse_fadt(&fadt).unwrap().sci_interrupt, 9);
    }

    #[test]
    fn rejects_truncated_sdt() {
        assert_eq!(parse_sdt_header(b"FACP"), Err(AcpiError::Truncated));
    }
}
