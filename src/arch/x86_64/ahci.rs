//! Minimal x86_64 AHCI boot path for discovering read-only SATA block devices.
//!
//! This is intentionally mechanism-only: it validates PCI BAR5, explicitly maps
//! the HBA MMIO window, initializes the controller/ports with bounded waits, and
//! exposes discovered read-only SATA geometry. Filesystem/root policy remains
//! above this architecture path.

use core::ptr::{read_volatile, write_volatile};

use mirage_platform::{
    PlatformDevice, PlatformLocation, PlatformPciBar, PlatformRegistry, MAX_PLATFORM_DEVICE_EVENTS,
};

use crate::kernel::device::{BlockStorageDevice, DeviceDriver, DeviceError, DeviceKind};
use crate::kernel::memory;
use crate::kernel::mmio::{map_mmio, verify_mapped, MmioFlags, MmioRegion, PhysAddr};
use crate::kernel::sync::SpinLock;
use crate::subkernel::{DeviceSecurity, SecurityClass};

const AHCI_BAR: usize = 5;
const DEFAULT_ABAR_SIZE: usize = 4096;
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

const GHC_HR: u32 = 1 << 0;
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
const ATA_WRITE_DMA_EXT: u8 = 0x35;
const ATA_FLUSH_CACHE_EXT: u8 = 0xea;
const ATA_IDENTIFY_PACKET_DEVICE: u8 = 0xa1;
const SATA_SIG_ATA: u32 = 0x0000_0101;
const SATA_SIG_ATAPI: u32 = 0xeb14_0101;
const SATA_SIG_SEMB: u32 = 0xc33c_0101;
const SATA_SIG_PM: u32 = 0x9669_0101;
const POLL_LIMIT: usize = 1_000_000;
const PCI_COMMAND_MEMORY: u16 = 1 << 1;
const PCI_COMMAND_BUS_MASTER: u16 = 1 << 2;

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
    NoDisk { atapi_detected: bool },
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AhciBarInfo {
    pub raw: u64,
    pub physical: u64,
    pub is_mmio: bool,
    pub is_64bit: bool,
    pub prefetchable: bool,
    pub size: usize,
}

pub fn ahci_bar_info(bar: PlatformPciBar, probed_size: usize) -> AhciBarInfo {
    AhciBarInfo {
        raw: bar.raw,
        physical: bar.base,
        is_mmio: bar.is_mmio,
        is_64bit: bar.is_64bit,
        prefetchable: bar.prefetchable,
        size: if probed_size == 0 {
            DEFAULT_ABAR_SIZE
        } else {
            probed_size
        },
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
pub static AHCI_SATA0_DRIVER: AhciSataBlockDriver = AhciSataBlockDriver;

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
        return AhciBootStatus::NoDisk {
            atapi_detected: false,
        };
    };
    let Some(hhdm) = hhdm_offset else {
        return AhciBootStatus::Failed("HHDM unavailable for AHCI DMA buffer access");
    };
    match unsafe { bring_up_device(device, hhdm) } {
        Ok(scan) => match scan.sata_disk {
            Some(info) => AhciBootStatus::Online(info),
            None => AhciBootStatus::NoDisk {
                atapi_detected: scan.atapi_detected,
            },
        },
        Err(reason) => AhciBootStatus::Failed(reason),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AhciScanResult {
    sata_disk: Option<RegisteredBlockInfo>,
    atapi_detected: bool,
}

unsafe fn bring_up_device(
    device: PlatformDevice,
    hhdm: u64,
) -> Result<AhciScanResult, &'static str> {
    let (bus, dev, function) = pci_location(device)?;
    crate::kprintln!(
        "[ahci] pci device: {:02x}:{:02x}.{} vendor={:04x} device={:04x}",
        bus,
        dev,
        function,
        device.vendor_id.unwrap_or(0xffff),
        device.device_id.unwrap_or(0xffff)
    );
    let command_before = pci_read16(bus, dev, function, 0x04);
    crate::kprintln!("[ahci] pci command before={:#06x}", command_before);

    let bar = device.bars[AHCI_BAR].ok_or("AHCI BAR5/ABAR missing")?;
    let bar_size =
        probe_bar_size(bus, dev, function, AHCI_BAR as u8, bar).unwrap_or(DEFAULT_ABAR_SIZE);
    let bar_info = ahci_bar_info(bar, bar_size);
    log_bar_info(bar_info);
    validate_bar(bar_info)?;

    let command_after = enable_pci_command(bus, dev, function)?;
    crate::kprintln!("[ahci] pci command after={:#06x}", command_after);

    let abar = map_mmio(
        PhysAddr(bar_info.physical),
        bar_info.size,
        MmioFlags::DEVICE,
    )
    .map_err(|_| "AHCI ABAR MMIO map failed")?;
    verify_mapped(abar.virt, core::cmp::min(0x14, abar.len), MmioFlags::DEVICE)
        .map_err(|_| "AHCI ABAR MMIO verification failed")?;
    crate::kprintln!(
        "[ahci] mapped ABAR phys={:#x} virt={:#x} len={:#x}",
        abar.phys.0,
        abar.virt.0,
        abar.len
    );
    print_page_walk(abar.virt.0);

    let hba = AhciHba::new(abar);
    let cap = hba.read32(HBA_CAP);
    let ghc = hba.read32(HBA_GHC);
    let pi = hba.read32(HBA_PI);
    let vs = hba.read32(HBA_VS);
    crate::kprintln!("[ahci] CAP={:#x}", cap);
    crate::kprintln!("[ahci] GHC={:#x}", ghc);
    crate::kprintln!("[ahci] PI={:#x}", pi);
    crate::kprintln!("[ahci] VS={:#x}", vs);

    if (ghc & GHC_AE) == 0 {
        hba.write32(HBA_GHC, ghc | GHC_AE);
    }
    let ghc_after_ae = hba.read32(HBA_GHC);
    if (ghc_after_ae & GHC_HR) != 0 {
        wait_clear(hba.base(), HBA_GHC, GHC_HR, "AHCI controller reset timeout")?;
    }

    let mut atapi_detected = false;
    let mut port = 0u8;
    while port < 32 {
        if (pi & (1u32 << port)) != 0 {
            crate::kprintln!("[ahci] port {} implemented", port);
            let base = port_base(port);
            let ssts = hba.read32(base + PX_SSTS);
            let status = parse_ssts(ssts);
            crate::kprintln!("[ahci] port {} ssts={:#x}", port, ssts);
            crate::kprintln!("[ahci] port {} det={} ipm={}", port, status.det, status.ipm);
            let sig = hba.read32(base + PX_SIG);
            let kind = classify_signature(sig);
            crate::kprintln!("[ahci] port {} sig={:#x}", port, sig);
            log_port_type(port, kind);
            if sata_device_present(ssts) && kind == PortSignature::SataDisk {
                crate::kprintln!("[ahci] port {} SATA disk detected", port);
                let info = init_port_and_identify(hba.base(), hhdm, port)?;
                return Ok(AhciScanResult {
                    sata_disk: Some(info),
                    atapi_detected,
                });
            } else if sata_device_present(ssts) && kind == PortSignature::Atapi {
                atapi_detected = true;
                crate::kprintln!("[ahci] port {} ATAPI device detected; packet media probing not yet enabled in this boot path", port);
            }
        }
        port += 1;
    }
    crate::kprintln!("[ahci] SATA Disk Skipped: no SATA disk detected");
    if atapi_detected {
        crate::kprintln!(
            "[ahci] ATAPI Detected; Optical Disk Skipped: packet media probe not enabled"
        );
    }
    Ok(AhciScanResult {
        sata_disk: None,
        atapi_detected,
    })
}

#[derive(Clone, Copy)]
struct AhciHba {
    region: MmioRegion,
}

impl AhciHba {
    const fn new(region: MmioRegion) -> Self {
        Self { region }
    }

    const fn base(self) -> usize {
        self.region.virt.0 as usize
    }

    unsafe fn read32(self, offset: usize) -> u32 {
        mmio_read32(self.base(), offset)
    }

    unsafe fn write32(self, offset: usize, value: u32) {
        mmio_write32(self.base(), offset, value)
    }
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
    crate::kprint!("[ahci] port {} identify ok\n[ahci] model=\"", port);
    print_model(&id.model);
    crate::kprintln!("\"");
    crate::kprintln!("[ahci] sectors={}", id.sectors);
    crate::kprintln!("[ahci] sector_size={}", id.sector_size);
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
    validate_read_request(disk.info, lba, count, buffer.len())?;
    let bytes = count as usize * disk.info.block_size as usize;
    if bytes > 4096 {
        return Err("read too large for AHCI bounce buffer");
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

pub fn write_blocks(
    _lba: u64,
    _count: u16,
    _buffer: &[u8],
    writes_enabled: bool,
) -> Result<(), &'static str> {
    let _disk = SATA0.lock().ok_or("sata0 not registered")?;
    if !writes_enabled {
        return Err("read-only: AHCI writes disabled by kernel policy");
    }
    Err("WRITE DMA EXT path requires explicit mount-rw integration")
}

pub fn flush() -> Result<(), &'static str> {
    let disk = SATA0.lock().ok_or("sata0 not registered")?;
    unsafe {
        issue_command(
            disk.mmio,
            disk.hhdm,
            port_base(disk.port),
            disk.command_list_phys,
            disk.command_table_phys,
            disk.dma_buffer_phys,
            ATA_FLUSH_CACHE_EXT,
            0,
            0,
            1,
        )
    }
}

pub fn validate_read_request(
    info: RegisteredBlockInfo,
    lba: u64,
    count: u16,
    buffer_len: usize,
) -> Result<(), &'static str> {
    if count == 0
        || lba
            .checked_add(count as u64)
            .is_none_or(|end| end > info.block_count)
    {
        return Err("read out of bounds");
    }
    let bytes = count as usize * info.block_size as usize;
    if buffer_len != bytes {
        return Err("invalid read buffer length");
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
    write_volatile(clv.add(0) as *mut u16, 5);
    write_volatile(clv.add(2) as *mut u16, 1);
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
    let is = mmio_read32(mmio, p + PX_IS);
    let tfd = mmio_read32(mmio, p + PX_TFD);
    if (tfd & TFD_ERR) != 0 || (is & 0x4000_0000) != 0 {
        return Err("AHCI command failed");
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

fn pci_location(device: PlatformDevice) -> Result<(u8, u8, u8), &'static str> {
    let PlatformLocation::Pci {
        bus,
        device,
        function,
    } = device.location
    else {
        return Err("AHCI platform device is not PCI");
    };
    Ok((bus, device, function))
}

fn log_bar_info(info: AhciBarInfo) {
    crate::kprintln!("[ahci] BAR5 raw={:#x}", info.raw);
    crate::kprintln!("[ahci] BAR5 physical={:#x}", info.physical);
    crate::kprintln!(
        "[ahci] BAR5 type={}",
        if info.is_mmio { "mmio" } else { "io" }
    );
    crate::kprintln!("[ahci] BAR5 width={}", if info.is_64bit { 64 } else { 32 });
    crate::kprintln!("[ahci] BAR5 prefetchable={}", info.prefetchable);
    crate::kprintln!("[ahci] BAR5 size={:#x}", info.size);
}

fn validate_bar(info: AhciBarInfo) -> Result<(), &'static str> {
    if !info.is_mmio {
        return Err("AHCI BAR5 is I/O port BAR, expected MMIO");
    }
    if info.physical == 0 {
        return Err("AHCI BAR5 physical address is zero");
    }
    if (info.physical & 0xfff) != 0 {
        return Err("AHCI BAR5 physical address is not page aligned");
    }
    if info.size < 0x110 {
        return Err("AHCI BAR5 size too small");
    }
    Ok(())
}

fn log_port_type(port: u8, kind: PortSignature) {
    let name = match kind {
        PortSignature::SataDisk => "SATA",
        PortSignature::Atapi => "ATAPI",
        PortSignature::Semb => "SEMB",
        PortSignature::PortMultiplier => "PortMultiplier",
        PortSignature::Unknown(_) => "unknown",
    };
    crate::kprintln!("[ahci] port {} type={}", port, name);
}

fn print_page_walk(virt: u64) {
    if let Some(walk) = crate::arch::x86_64::paging::walk_kernel_page_tables(virt) {
        crate::kprintln!(
            "[ahci] ABAR page walk cr3={:#x} pml4e={:#x} pdpte={:#x} pde={:#x} pte={:#x}",
            walk.cr3,
            walk.pml4e,
            walk.pdpte,
            walk.pde,
            walk.pte
        );
    }
}

fn pci_config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    0x8000_0000u32
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | ((offset as u32) & 0xfc)
}

unsafe fn pci_read32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    crate::arch::x86_64::io::outl(0xcf8, pci_config_address(bus, device, function, offset));
    crate::arch::x86_64::io::inl(0xcfc)
}

unsafe fn pci_write32(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    crate::arch::x86_64::io::outl(0xcf8, pci_config_address(bus, device, function, offset));
    crate::arch::x86_64::io::outl(0xcfc, value);
}

unsafe fn pci_read16(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
    let value = pci_read32(bus, device, function, offset & !0x3);
    ((value >> ((offset & 0x2) * 8)) & 0xffff) as u16
}

unsafe fn pci_write16(bus: u8, device: u8, function: u8, offset: u8, value: u16) {
    let aligned = offset & !0x3;
    let shift = (offset & 0x2) * 8;
    let old = pci_read32(bus, device, function, aligned);
    let new = (old & !(0xffff << shift)) | ((value as u32) << shift);
    pci_write32(bus, device, function, aligned, new);
}

unsafe fn enable_pci_command(bus: u8, device: u8, function: u8) -> Result<u16, &'static str> {
    let command =
        pci_read16(bus, device, function, 0x04) | PCI_COMMAND_MEMORY | PCI_COMMAND_BUS_MASTER;
    pci_write16(bus, device, function, 0x04, command);
    let after = pci_read16(bus, device, function, 0x04);
    if (after & (PCI_COMMAND_MEMORY | PCI_COMMAND_BUS_MASTER))
        != (PCI_COMMAND_MEMORY | PCI_COMMAND_BUS_MASTER)
    {
        return Err("AHCI PCI command bits did not stick");
    }
    Ok(after)
}

unsafe fn probe_bar_size(
    bus: u8,
    device: u8,
    function: u8,
    index: u8,
    bar: PlatformPciBar,
) -> Option<usize> {
    let offset = 0x10 + index * 4;
    let old_low = pci_read32(bus, device, function, offset);
    let old_high = if bar.is_64bit && index < 5 {
        Some(pci_read32(bus, device, function, offset + 4))
    } else {
        None
    };
    pci_write32(bus, device, function, offset, 0xffff_ffff);
    if old_high.is_some() {
        pci_write32(bus, device, function, offset + 4, 0xffff_ffff);
    }
    let mask_low = pci_read32(bus, device, function, offset);
    let mask_high = if old_high.is_some() {
        Some(pci_read32(bus, device, function, offset + 4))
    } else {
        None
    };
    pci_write32(bus, device, function, offset, old_low);
    if let Some(high) = old_high {
        pci_write32(bus, device, function, offset + 4, high);
    }
    let size = if bar.is_mmio && bar.is_64bit {
        let mask =
            (((mask_high.unwrap_or(0) as u64) << 32) | (mask_low as u64)) & 0xffff_ffff_ffff_fff0;
        if mask == 0 {
            return None;
        }
        (!mask).wrapping_add(1)
    } else if bar.is_mmio {
        let mask = mask_low & 0xffff_fff0;
        if mask == 0 {
            return None;
        }
        (!(mask as u32)).wrapping_add(1) as u64
    } else {
        let mask = mask_low & 0xffff_fffc;
        if mask == 0 {
            return None;
        }
        (!(mask as u32)).wrapping_add(1) as u64
    };
    if size == 0 || size > (usize::MAX as u64) {
        None
    } else {
        Some(size as usize)
    }
}

pub struct AhciSataBlockDriver;

impl DeviceDriver for AhciSataBlockDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::BlockStorage
    }
    fn name(&self) -> &'static str {
        "sata0"
    }
    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Confidential, true)
    }
    fn as_block_storage(&self) -> Option<&dyn BlockStorageDevice> {
        Some(self)
    }
}

impl BlockStorageDevice for AhciSataBlockDriver {
    fn sector_size(&self) -> usize {
        lookup_by_name("sata0")
            .map(|i| i.block_size as usize)
            .unwrap_or(512)
    }
    fn sector_count(&self) -> u64 {
        lookup_by_name("sata0").map(|i| i.block_count).unwrap_or(0)
    }
    fn read_sectors(&self, first_sector: u64, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        let info = lookup_by_name("sata0").ok_or(DeviceError::NotFound)?;
        if buffer.len() % info.block_size as usize != 0 {
            return Err(DeviceError::BufferTooSmall);
        }
        let count = buffer.len() / info.block_size as usize;
        if count == 0 || count > u16::MAX as usize {
            return Err(DeviceError::Unsupported);
        }
        read_blocks(first_sector, count as u16, buffer).map_err(|_| DeviceError::Unsupported)?;
        Ok(buffer.len())
    }
    fn write_sectors(&self, _first_sector: u64, _data: &[u8]) -> Result<usize, DeviceError> {
        Err(DeviceError::Unsupported)
    }
    fn flush(&self) -> Result<(), DeviceError> {
        flush().map_err(|_| DeviceError::Unsupported)
    }
    fn discard(&self, _first_sector: u64, _sector_count: u64) -> Result<(), DeviceError> {
        Err(DeviceError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar5_parsing_uses_mmio_metadata_and_default_size() {
        let bar = PlatformPciBar::mmio32(5, 0xfebd_5000);
        let info = ahci_bar_info(bar, 0);
        assert!(info.is_mmio);
        assert_eq!(info.physical, 0xfebd_5000);
        assert_eq!(info.size, DEFAULT_ABAR_SIZE);
    }

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
    fn read_blocks_bounds_validation() {
        let info = RegisteredBlockInfo {
            name: "sata0",
            block_count: 4,
            block_size: 512,
            readonly: true,
        };
        assert_eq!(validate_read_request(info, 0, 1, 512), Ok(()));
        assert_eq!(
            validate_read_request(info, 4, 1, 512),
            Err("read out of bounds")
        );
        assert_eq!(
            validate_read_request(info, 0, 1, 256),
            Err("invalid read buffer length")
        );
    }

    #[test]
    fn block_device_registration_state_reports_absent_by_default() {
        *SATA0.lock() = None;
        assert_eq!(lookup_by_name("sata0"), None);
        let mut buffer = [0u8; 512];
        assert_eq!(read_blocks(0, 1, &mut buffer), Err("sata0 not registered"));
    }
}
