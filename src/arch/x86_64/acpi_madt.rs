//! Minimal ACPI MADT discovery for x86_64 hardware interrupt bring-up.

use core::{mem, ptr};
use crate::arch::x86_64::boot::BootInfo;

const RSDP_SIGNATURE: &[u8; 8] = b"RSD PTR ";
const MADT_SIGNATURE: &[u8; 4] = b"APIC";
const MAX_LOCAL_APICS: usize = 32;
const MAX_IO_APICS: usize = 8;
const MAX_ISO: usize = 16;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct RsdpV1 { signature: [u8; 8], checksum: u8, oem_id: [u8; 6], revision: u8, rsdt_address: u32 }
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct RsdpV2 { v1: RsdpV1, length: u32, xsdt_address: u64, extended_checksum: u8, reserved: [u8; 3] }
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct SdtHeader { signature: [u8; 4], length: u32, revision: u8, checksum: u8, oem_id: [u8; 6], oem_table_id: [u8; 8], oem_revision: u32, creator_id: u32, creator_revision: u32 }
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct MadtHeader { sdt: SdtHeader, local_apic_address: u32, flags: u32 }

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MadtLocalApic { pub processor_id: u8, pub apic_id: u8, pub flags: u32 }
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MadtIoApic { pub id: u8, pub address: u32, pub gsi_base: u32 }
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MadtInterruptOverride { pub bus: u8, pub source_irq: u8, pub gsi: u32, pub flags: u16 }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MadtInfo {
    pub local_apic_address: u64,
    pub flags: u32,
    local_apics: [Option<MadtLocalApic>; MAX_LOCAL_APICS],
    local_apic_count: usize,
    io_apics: [Option<MadtIoApic>; MAX_IO_APICS],
    io_apic_count: usize,
    overrides: [Option<MadtInterruptOverride>; MAX_ISO],
    override_count: usize,
}

impl MadtInfo {
    pub const fn empty(local_apic_address: u64, flags: u32) -> Self { Self { local_apic_address, flags, local_apics: [None; MAX_LOCAL_APICS], local_apic_count: 0, io_apics: [None; MAX_IO_APICS], io_apic_count: 0, overrides: [None; MAX_ISO], override_count: 0 } }
    pub const fn local_apics(&self) -> &[Option<MadtLocalApic>; MAX_LOCAL_APICS] { &self.local_apics }
    pub const fn local_apic_count(&self) -> usize { self.local_apic_count }
    pub const fn io_apics(&self) -> &[Option<MadtIoApic>; MAX_IO_APICS] { &self.io_apics }
    pub const fn io_apic_count(&self) -> usize { self.io_apic_count }
    pub const fn interrupt_overrides(&self) -> &[Option<MadtInterruptOverride>; MAX_ISO] { &self.overrides }
    pub const fn interrupt_override_count(&self) -> usize { self.override_count }
    pub fn override_for_irq(&self, irq: u8) -> Option<MadtInterruptOverride> { let mut i=0; while i<self.override_count { if let Some(e)=self.overrides[i] { if e.source_irq==irq { return Some(e); } } i+=1; } None }
    fn push_local_apic(&mut self, e: MadtLocalApic) { if self.local_apic_count<MAX_LOCAL_APICS { self.local_apics[self.local_apic_count]=Some(e); self.local_apic_count+=1; } }
    fn push_io_apic(&mut self, e: MadtIoApic) { if self.io_apic_count<MAX_IO_APICS { self.io_apics[self.io_apic_count]=Some(e); self.io_apic_count+=1; } }
    fn push_override(&mut self, e: MadtInterruptOverride) { if self.override_count<MAX_ISO { self.overrides[self.override_count]=Some(e); self.override_count+=1; } }
}

pub fn discover(boot_info: &BootInfo) -> Option<MadtInfo> { unsafe { discover_inner(boot_info.rsdp?.0, boot_info.hhdm_offset?) } }
unsafe fn discover_inner(rsdp_physical: u64, hhdm: u64) -> Option<MadtInfo> {
    let rsdp = phys_ptr::<RsdpV1>(rsdp_physical, hhdm); let r1 = read_unaligned(rsdp)?;
    if &r1.signature != RSDP_SIGNATURE || !checksum_ok(rsdp as *const u8, mem::size_of::<RsdpV1>()) { return None; }
    let mut root = r1.rsdt_address as u64; let mut xsdt=false;
    if r1.revision >= 2 { let r2 = read_unaligned(phys_ptr::<RsdpV2>(rsdp_physical,hhdm))?; if r2.length as usize >= mem::size_of::<RsdpV2>() && checksum_ok(rsdp as *const u8, r2.length as usize) && r2.xsdt_address != 0 { root=r2.xsdt_address; xsdt=true; } }
    find_madt(root,hhdm,xsdt)
}
unsafe fn find_madt(root_physical: u64, hhdm: u64, xsdt: bool) -> Option<MadtInfo> {
    let hp = phys_ptr::<SdtHeader>(root_physical,hhdm); let h = read_unaligned(hp)?; let len=h.length as usize;
    if len < mem::size_of::<SdtHeader>() || !checksum_ok(hp as *const u8,len) { return None; }
    let es = if xsdt {8usize} else {4usize}; let count=(len-mem::size_of::<SdtHeader>())/es; let base=(hp as usize+mem::size_of::<SdtHeader>()) as *const u8;
    let mut i=0; while i<count { let phys=if xsdt { ptr::read_unaligned(base.add(i*es) as *const u64) } else { ptr::read_unaligned(base.add(i*es) as *const u32) as u64 }; let thp=phys_ptr::<SdtHeader>(phys,hhdm); if let Some(th)=read_unaligned(thp) { let tl=th.length as usize; if th.signature == *MADT_SIGNATURE && tl >= mem::size_of::<MadtHeader>() && checksum_ok(thp as *const u8,tl) { return parse_madt(phys,hhdm); } } i+=1; } None
}
unsafe fn parse_madt(madt_physical: u64, hhdm: u64) -> Option<MadtInfo> {
    let mp = phys_ptr::<MadtHeader>(madt_physical,hhdm); let m=read_unaligned(mp)?; let table_len=m.sdt.length as usize; let mut info=MadtInfo::empty(m.local_apic_address as u64,m.flags); let mut off=mem::size_of::<MadtHeader>(); let base=mp as *const u8;
    while off+2 <= table_len { let ty=ptr::read_unaligned(base.add(off)); let len=ptr::read_unaligned(base.add(off+1)) as usize; if len<2 || off+len>table_len { break; } match ty { 0 if len>=8 => info.push_local_apic(MadtLocalApic{processor_id:ptr::read_unaligned(base.add(off+2)),apic_id:ptr::read_unaligned(base.add(off+3)),flags:ptr::read_unaligned(base.add(off+4) as *const u32)}), 1 if len>=12 => info.push_io_apic(MadtIoApic{id:ptr::read_unaligned(base.add(off+2)),address:ptr::read_unaligned(base.add(off+4) as *const u32),gsi_base:ptr::read_unaligned(base.add(off+8) as *const u32)}), 2 if len>=10 => info.push_override(MadtInterruptOverride{bus:ptr::read_unaligned(base.add(off+2)),source_irq:ptr::read_unaligned(base.add(off+3)),gsi:ptr::read_unaligned(base.add(off+4) as *const u32),flags:ptr::read_unaligned(base.add(off+8) as *const u16)}), 5 if len>=12 => { let addr=ptr::read_unaligned(base.add(off+4) as *const u64); if addr!=0 { info.local_apic_address=addr; } }, _=>{} } off+=len; } Some(info)
}
unsafe fn read_unaligned<T: Copy>(p: *const T) -> Option<T> { if p.is_null(){None}else{Some(ptr::read_unaligned(p))} }
const fn phys_ptr<T>(physical: u64, hhdm: u64) -> *const T { physical.saturating_add(hhdm) as *const T }
unsafe fn checksum_ok(p: *const u8, len: usize) -> bool { let mut sum=0u8; let mut i=0; while i<len { sum=sum.wrapping_add(ptr::read_unaligned(p.add(i))); i+=1; } sum==0 }
