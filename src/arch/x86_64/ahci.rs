//! Minimal x86_64 AHCI boot path for discovering one read-only SATA block device.
//!
//! This is intentionally mechanism-only: it initializes the HBA, identifies disks,
//! and exposes the discovered block geometry. Filesystem/root policy remains above
//! this architecture path.

use core::ptr::{read_volatile, write_volatile};

use mirage_platform::{
    PlatformDevice, PlatformLocation, PlatformRegistry, MAX_PLATFORM_DEVICE_EVENTS,
};

use crate::kernel::memory;
use crate::kernel::sync::SpinLock;

const AHCI_BAR: usize = 5;
const HBA_CAP: usize = 0x00;
const HBA_GHC: usize = 0x04;
const HBA_PI: usize = 0x0c;
const HBA_VS: usize = 0x10;
const HBA_PORTS: usize = 0x100;
const PORT_STRIDE: usize = 0x80;
const PX_CLB: usize = 0x00;
const PX_CLBU: usize = 0x04;
const PX_FB: usize = 0x08;
const PX_FBU: usize = 0x0c;
const PX_IS: usize = 0x10;
const PX_CMD: usize = 0x18;
const PX_TFD: usize = 0x20;
const PX_SIG: usize = 0x24;
const PX_SSTS: usize = 0x28;
const PX_SERR: usize = 0x30;
const PX_CI: usize = 0x38;

const GHC_AE: u32 = 1 << 31;
const CMD_ST: u32 = 1 << 0;
const CMD_FRE: u32 = 1 << 4;
const CMD_FR: u32 = 1 << 14;
const CMD_CR: u32 = 1 << 15;
const TFD_BSY: u32 = 1 << 7;
const TFD_DRQ: u32 = 1 << 3;
const TFD_ERR: u32 = 1 << 0;
const ATA_IDENTIFY_DEVICE: u8 = 0xec;
const ATA_READ_DMA_EXT: u8 = 0x25;
const SATA_SIG_ATA: u32 = 0x0000_0101;
const SATA_SIG_ATAPI: u32 = 0xeb14_0101;
const SATA_SIG_SEMB: u32 = 0xc33c_0101;
const SATA_SIG_PM: u32 = 0x9669_0101;
const POLL_LIMIT: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegisteredBlockInfo {
    pub name: &'static str,
    pub block_count: u64,
    pub block_size: u32,
    pub readonly: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AhciBootStatus {
    Online(RegisteredBlockInfo),
    NoDisk,
    Failed(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortSignature {
    SataDisk,
    Atapi,
    Semb,
    PortMultiplier,
    Unknown(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SataStatus {
    pub det: u8,
    pub ipm: u8,
}

pub const fn parse_ssts(raw: u32) -> SataStatus {
    SataStatus {
        det: (raw & 0x0f) as u8,
        ipm: ((raw >> 8) & 0x0f) as u8,
    }
}

pub const fn sata_device_present(raw: u32) -> bool {
    let s = parse_ssts(raw);
    s.det == 3 && s.ipm == 1
}

pub const fn classify_signature(raw: u32) -> PortSignature {
    match raw {
        SATA_SIG_ATA => PortSignature::SataDisk,
        SATA_SIG_ATAPI => PortSignature::Atapi,
        SATA_SIG_SEMB => PortSignature::Semb,
        SATA_SIG_PM => PortSignature::PortMultiplier,
        other => PortSignature::Unknown(other),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentifyInfo {
    pub sectors: u64,
    pub sector_size: u32,
    pub model: [u8; 40],
    pub serial: [u8; 20],
    pub firmware: [u8; 8],
}

pub fn parse_identify(words: &[u16; 256]) -> IdentifyInfo {
    let mut model = [0u8; 40];
    let mut serial = [0u8; 20];
    let mut firmware = [0u8; 8];
    copy_ata_string(words, 10, &mut serial);
    copy_ata_string(words, 23, &mut firmware);
    copy_ata_string(words, 27, &mut model);
    let lba48 = (words[83] & (1 << 10)) != 0;
    let sectors = if lba48 {
        (words[100] as u64)
            | ((words[101] as u64) << 16)
            | ((words[102] as u64) << 32)
            | ((words[103] as u64) << 48)
    } else {
        (words[60] as u64) | ((words[61] as u64) << 16)
    };
    let sector_size = if (words[106] & (1 << 14)) != 0
        && (words[106] & (1 << 15)) == 0
        && (words[106] & (1 << 12)) != 0
    {
        ((words[117] as u32) | ((words[118] as u32) << 16)).saturating_mul(2)
    } else {
        512
    };
    IdentifyInfo {
        sectors,
        sector_size,
        model,
        serial,
        firmware,
    }
}

fn copy_ata_string(words: &[u16; 256], start: usize, out: &mut [u8]) {
    let mut i = 0usize;
    while i < out.len() / 2 {
        let w = words[start + i];
        out[i * 2] = (w >> 8) as u8;
        out[i * 2 + 1] = w as u8;
        i += 1;
    }
    let mut end = out.len();
    while end > 0 && out[end - 1] == b' ' {
        out[end - 1] = 0;
        end -= 1;
    }
}

#[derive(Clone, Copy)]
struct AhciDiskState {
    mmio: usize,
    hhdm: u64,
    port: u8,
    command_list_phys: u64,
    _fis_phys: u64,
    command_table_phys: u64,
    dma_buffer_phys: u64,
    info: RegisteredBlockInfo,
}

static SATA0: SpinLock<Option<AhciDiskState>> = SpinLock::new(None);

pub fn lookup_by_name(name: &str) -> Option<RegisteredBlockInfo> {
    if name == "sata0" {
        SATA0.lock().map(|d| d.info)
    } else {
        None
    }
}

pub fn bring_up_first_sata_disk(
    platform: &PlatformRegistry<MAX_PLATFORM_DEVICE_EVENTS>,
    hhdm_offset: Option<u64>,
) -> AhciBootStatus {
    let Some(device) = platform.platform_find_ahci_controller() else {
        return AhciBootStatus::NoDisk;
    };
    let Some(hhdm) = hhdm_offset else {
        return AhciBootStatus::Failed("HHDM unavailable for AHCI MMIO mapping");
    };
    match unsafe { bring_up_device(device, hhdm) } {
        Ok(Some(info)) => AhciBootStatus::Online(info),
        Ok(None) => AhciBootStatus::NoDisk,
        Err(reason) => AhciBootStatus::Failed(reason),
    }
}

unsafe fn bring_up_device(
    device: PlatformDevice,
    hhdm: u64,
) -> Result<Option<RegisteredBlockInfo>, &'static str> {
    enable_pci_command(device)?;
    let bar = device
        .mmio_bar(AHCI_BAR)
        .ok_or("AHCI BAR5/ABAR missing or not MMIO")?;
    let mmio = hhdm
        .checked_add(bar.base)
        .ok_or("AHCI ABAR mapping overflow")? as usize;
    let cap = mmio_read32(mmio, HBA_CAP);
    let ghc = mmio_read32(mmio, HBA_GHC);
    let pi = mmio_read32(mmio, HBA_PI);
    let vs = mmio_read32(mmio, HBA_VS);
    crate::kprintln!("[ahci] abar={:#x}", bar.base);
    crate::kprintln!("[ahci] cap={:#x} ghc={:#x}", cap, ghc);
    crate::kprintln!("[ahci] pi={:#x}", pi);
    crate::kprintln!("[ahci] version={:#x}", vs);
    mmio_write32(mmio, HBA_GHC, ghc | GHC_AE);

    let mut port = 0u8;
    while port < 32 {
        if (pi & (1u32 << port)) != 0 {
            crate::kprintln!("[ahci] port {} implemented", port);
            let base = port_base(port);
            let ssts = mmio_read32(mmio, base + PX_SSTS);
            crate::kprintln!("[ahci] port {} ssts={:#x}", port, ssts);
            if sata_device_present(ssts) {
                let sig = mmio_read32(mmio, base + PX_SIG);
                crate::kprintln!("[ahci] port {} sig={:#x}", port, sig);
                if classify_signature(sig) == PortSignature::SataDisk {
                    crate::kprintln!("[ahci] port {} SATA disk detected", port);
                    let info = init_port_and_identify(mmio, hhdm, port)?;
                    return Ok(Some(info));
                }
            }
        }
        port += 1;
    }
    Ok(None)
}

unsafe fn init_port_and_identify(
    mmio: usize,
    hhdm: u64,
    port: u8,
) -> Result<RegisteredBlockInfo, &'static str> {
    let cl = alloc_frame()?;
    let fis = alloc_frame()?;
    let ct = alloc_frame()?;
    let buf = alloc_frame()?;
    zero_frame(hhdm, cl);
    zero_frame(hhdm, fis);
    zero_frame(hhdm, ct);
    zero_frame(hhdm, buf);
    let p = port_base(port);
    stop_port(mmio, p)?;
    mmio_write32(mmio, p + PX_CLB, cl as u32);
    mmio_write32(mmio, p + PX_CLBU, (cl >> 32) as u32);
    mmio_write32(mmio, p + PX_FB, fis as u32);
    mmio_write32(mmio, p + PX_FBU, (fis >> 32) as u32);
    mmio_write32(mmio, p + PX_SERR, u32::MAX);
    mmio_write32(mmio, p + PX_IS, u32::MAX);
    start_port(mmio, p);
    issue_command(mmio, hhdm, p, cl, ct, buf, ATA_IDENTIFY_DEVICE, 0, 1, 512)?;
    let words = core::slice::from_raw_parts((hhdm + buf) as *const u16, 256);
    let mut copy = [0u16; 256];
    copy.copy_from_slice(words);
    let id = parse_identify(&copy);
    if id.sectors == 0 {
        return Err("ATA IDENTIFY returned zero sectors");
    }
    crate::kprint!("[ahci] port {} identify ok: model=\"", port);
    print_model(&id.model);
    crate::kprintln!("\" sectors={} sector_size={}", id.sectors, id.sector_size);
    let info = RegisteredBlockInfo {
        name: "sata0",
        block_count: id.sectors,
        block_size: id.sector_size,
        readonly: true,
    };
    *SATA0.lock() = Some(AhciDiskState {
        mmio,
        hhdm,
        port,
        command_list_phys: cl,
        _fis_phys: fis,
        command_table_phys: ct,
        dma_buffer_phys: buf,
        info,
    });
    Ok(info)
}

pub fn read_blocks(lba: u64, count: u16, buffer: &mut [u8]) -> Result<(), &'static str> {
    let disk = SATA0.lock().ok_or("sata0 not registered")?;
    if count == 0
        || lba
            .checked_add(count as u64)
            .is_none_or(|end| end > disk.info.block_count)
    {
        return Err("read out of bounds");
    }
    let bytes = count as usize * disk.info.block_size as usize;
    if buffer.len() != bytes || bytes > 4096 {
        return Err("invalid read buffer length");
    }
    unsafe {
        zero_frame(disk.hhdm, disk.dma_buffer_phys);
        issue_command(
            disk.mmio,
            disk.hhdm,
            port_base(disk.port),
            disk.command_list_phys,
            disk.command_table_phys,
            disk.dma_buffer_phys,
            ATA_READ_DMA_EXT,
            lba,
            count,
            bytes as u32,
        )?;
        let src =
            core::slice::from_raw_parts((disk.hhdm + disk.dma_buffer_phys) as *const u8, bytes);
        buffer.copy_from_slice(src);
    }
    Ok(())
}

unsafe fn issue_command(
    mmio: usize,
    hhdm: u64,
    p: usize,
    cl: u64,
    ct: u64,
    data: u64,
    cmd: u8,
    lba: u64,
    sectors: u16,
    bytes: u32,
) -> Result<(), &'static str> {
    wait_clear(mmio, p + PX_TFD, TFD_BSY | TFD_DRQ, "AHCI task file busy")?;
    zero_frame(hhdm, ct);
    let clv = (hhdm + cl) as *mut u8;
    write_volatile(clv.add(0) as *mut u16, 5); // CFL=5, read command
    write_volatile(clv.add(2) as *mut u16, 1); // one PRDT
    write_volatile(clv.add(4) as *mut u32, 0);
    write_volatile(clv.add(8) as *mut u32, ct as u32);
    write_volatile(clv.add(12) as *mut u32, (ct >> 32) as u32);
    let ctv = (hhdm + ct) as *mut u8;
    write_fis(ctv, cmd, lba, sectors);
    let prdt = ctv.add(0x80);
    write_volatile(prdt as *mut u32, data as u32);
    write_volatile(prdt.add(4) as *mut u32, (data >> 32) as u32);
    write_volatile(prdt.add(8) as *mut u32, 0);
    write_volatile(prdt.add(12) as *mut u32, (bytes - 1) | (1 << 31));
    mmio_write32(mmio, p + PX_IS, u32::MAX);
    mmio_write32(mmio, p + PX_CI, 1);
    wait_clear(mmio, p + PX_CI, 1, "AHCI command completion")?;
    let tfd = mmio_read32(mmio, p + PX_TFD);
    if (tfd & TFD_ERR) != 0 {
        return Err("AHCI command task-file error");
    }
    Ok(())
}

unsafe fn write_fis(ptr: *mut u8, command: u8, lba: u64, sectors: u16) {
    let mut fis = [0u8; 20];
    fis[0] = 0x27;
    fis[1] = 0x80;
    fis[2] = command;
    fis[4] = lba as u8;
    fis[5] = (lba >> 8) as u8;
    fis[6] = (lba >> 16) as u8;
    fis[7] = 1 << 6;
    fis[8] = (lba >> 24) as u8;
    fis[9] = (lba >> 32) as u8;
    fis[10] = (lba >> 40) as u8;
    fis[12] = sectors as u8;
    fis[13] = (sectors >> 8) as u8;
    let mut i = 0usize;
    while i < fis.len() {
        write_volatile(ptr.add(i), fis[i]);
        i += 1;
    }
}

unsafe fn stop_port(mmio: usize, p: usize) -> Result<(), &'static str> {
    let cmd = mmio_read32(mmio, p + PX_CMD) & !(CMD_ST | CMD_FRE);
    mmio_write32(mmio, p + PX_CMD, cmd);
    wait_clear(mmio, p + PX_CMD, CMD_CR | CMD_FR, "AHCI port stop")
}

unsafe fn start_port(mmio: usize, p: usize) {
    let cmd = mmio_read32(mmio, p + PX_CMD) | CMD_FRE | CMD_ST;
    mmio_write32(mmio, p + PX_CMD, cmd);
}

unsafe fn wait_clear(
    mmio: usize,
    offset: usize,
    mask: u32,
    reason: &'static str,
) -> Result<(), &'static str> {
    let mut i = 0usize;
    while i < POLL_LIMIT {
        if (mmio_read32(mmio, offset) & mask) == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
        i += 1;
    }
    Err(reason)
}

unsafe fn mmio_read32(base: usize, offset: usize) -> u32 {
    read_volatile((base + offset) as *const u32)
}
unsafe fn mmio_write32(base: usize, offset: usize, value: u32) {
    write_volatile((base + offset) as *mut u32, value)
}
const fn port_base(port: u8) -> usize {
    HBA_PORTS + (port as usize * PORT_STRIDE)
}

fn alloc_frame() -> Result<u64, &'static str> {
    memory::allocate_physical_frame().ok_or("AHCI DMA allocation failed")
}

unsafe fn zero_frame(hhdm: u64, phys: u64) {
    if phys == 0 && hhdm == 0 {
        return;
    }
    let ptr = (hhdm + phys) as *mut u8;
    let mut i = 0usize;
    while i < 4096 {
        write_volatile(ptr.add(i), 0);
        i += 1;
    }
}

fn print_model(model: &[u8; 40]) {
    for &b in model.iter() {
        if b == 0 {
            break;
        }
        crate::kprint!("{}", b as char);
    }
}

fn enable_pci_command(device: PlatformDevice) -> Result<(), &'static str> {
    let PlatformLocation::Pci {
        bus,
        device,
        function,
    } = device.location
    else {
        return Err("AHCI platform device is not PCI");
    };
    let address = |offset: u8| -> u32 {
        0x8000_0000u32
            | ((bus as u32) << 16)
            | ((device as u32) << 11)
            | ((function as u32) << 8)
            | ((offset as u32) & 0xfc)
    };
    unsafe {
        crate::arch::x86_64::io::outl(0xcf8, address(0x04));
        let value = crate::arch::x86_64::io::inl(0xcfc) | 0x0006;
        crate::arch::x86_64::io::outl(0xcf8, address(0x04));
        crate::arch::x86_64::io::outl(0xcfc, value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pxssts_parsing_detects_active_device() {
        assert_eq!(parse_ssts(0x133), SataStatus { det: 3, ipm: 1 });
        assert!(sata_device_present(0x133));
        assert!(!sata_device_present(0x303));
    }

    #[test]
    fn signature_classification() {
        assert_eq!(classify_signature(SATA_SIG_ATA), PortSignature::SataDisk);
        assert_eq!(classify_signature(SATA_SIG_ATAPI), PortSignature::Atapi);
        assert_eq!(classify_signature(SATA_SIG_SEMB), PortSignature::Semb);
        assert_eq!(
            classify_signature(SATA_SIG_PM),
            PortSignature::PortMultiplier
        );
        assert_eq!(
            classify_signature(0xdead_beef),
            PortSignature::Unknown(0xdead_beef)
        );
    }

    #[test]
    fn identify_sector_count_and_string_parsing() {
        let mut words = [0u16; 256];
        words[83] = 1 << 10;
        words[100] = 0x3456;
        words[101] = 0x0012;
        let text = b"MIRAGE SATA DISK                    ";
        for (i, chunk) in text.chunks(2).enumerate() {
            words[27 + i] = ((chunk[0] as u16) << 8) | chunk[1] as u16;
        }
        let id = parse_identify(&words);
        assert_eq!(id.sectors, 0x0012_3456);
        assert_eq!(&id.model[..16], b"MIRAGE SATA DISK");
        assert_eq!(id.sector_size, 512);
    }
    #[test]
    fn read_blocks_rejects_unregistered_sata0() {
        *SATA0.lock() = None;
        let mut buffer = [0u8; 512];
        assert_eq!(read_blocks(0, 1, &mut buffer), Err("sata0 not registered"));
    }
}
