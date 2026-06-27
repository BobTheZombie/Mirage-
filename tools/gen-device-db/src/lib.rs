use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Default)]
pub struct Database {
    pub cpus: Vec<Cpu>,
    pub pci_vendors: Vec<Vendor>,
    pub pci_devices: Vec<PciDevice>,
    pub pci_classes: Vec<Class>,
    pub usb_vendors: Vec<Vendor>,
    pub usb_devices: Vec<UsbDevice>,
    pub usb_classes: Vec<Class>,
    pub block: Vec<Block>,
    pub input: Vec<Simple>,
    pub char_devs: Vec<Simple>,
    pub chipsets: Vec<Chipset>,
    pub aliases: BTreeMap<String, BTreeMap<String, String>>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Cpu {
    pub vendor: String,
    pub family: u16,
    pub model: Option<u16>,
    pub stepping: Option<u8>,
    pub name: String,
    pub codename: Option<String>,
    #[serde(default)]
    pub driver_hints: Vec<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Vendor {
    pub id: u16,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct PciDevice {
    pub vendor_id: u16,
    pub device_id: u16,
    pub name: String,
    pub driver_hint: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct UsbDevice {
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: String,
    pub driver_hint: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Class {
    pub class: u8,
    pub subclass: Option<u8>,
    pub prog_if: Option<u8>,
    pub name: String,
    pub driver_hint: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Block {
    pub kind: String,
    pub name: String,
    pub driver_hint: String,
    pub default_block_size: u32,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Simple {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub driver_hints: Vec<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Chipset {
    pub family: String,
    pub name: String,
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default)]
    pub driver_hints: Vec<String>,
}
#[derive(Deserialize)]
struct CpuFile {
    #[serde(default)]
    cpu: Vec<Cpu>,
}
#[derive(Deserialize)]
struct VendorFile {
    #[serde(default)]
    vendor: Vec<Vendor>,
}
#[derive(Deserialize)]
struct PciDeviceFile {
    #[serde(default)]
    device: Vec<PciDevice>,
}
#[derive(Deserialize)]
struct UsbDeviceFile {
    #[serde(default)]
    device: Vec<UsbDevice>,
}
#[derive(Deserialize)]
struct ClassFile {
    #[serde(default)]
    class: Vec<Class>,
}
#[derive(Deserialize)]
struct BlockFile {
    #[serde(default)]
    block: Vec<Block>,
}
#[derive(Deserialize)]
struct InputFile {
    #[serde(default)]
    input: Vec<Simple>,
}
#[derive(Deserialize)]
struct CharFile {
    #[serde(default, rename = "char")]
    char_devs: Vec<Simple>,
}
#[derive(Deserialize)]
struct ChipsetFile {
    #[serde(default)]
    chipset: Vec<Chipset>,
}
#[derive(Deserialize)]
struct AliasFile {
    aliases: BTreeMap<String, BTreeMap<String, String>>,
}

pub fn load_database(root: impl AsRef<Path>) -> Result<Database, String> {
    let root = root.as_ref();
    let mut db = Database::default();
    for path in sorted_toml_files(root)? {
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        validate_hex_widths(&rel, &text)?;
        match rel.as_str() {
            "pci/vendors.toml" => db.pci_vendors = from::<VendorFile>(&path, &text)?.vendor,
            "pci/devices.toml" => db.pci_devices = from::<PciDeviceFile>(&path, &text)?.device,
            "pci/classes.toml" => db.pci_classes = from::<ClassFile>(&path, &text)?.class,
            "usb/vendors.toml" => db.usb_vendors = from::<VendorFile>(&path, &text)?.vendor,
            "usb/devices.toml" => db.usb_devices = from::<UsbDeviceFile>(&path, &text)?.device,
            "usb/classes.toml" => db.usb_classes = from::<ClassFile>(&path, &text)?.class,
            "block/classes.toml" => db.block = from::<BlockFile>(&path, &text)?.block,
            "input/classes.toml" => db.input = from::<InputFile>(&path, &text)?.input,
            "char/classes.toml" => db.char_devs = from::<CharFile>(&path, &text)?.char_devs,
            "aliases.toml" => db.aliases = from::<AliasFile>(&path, &text)?.aliases,
            r if r.starts_with("cpu/") => db.cpus.extend(from::<CpuFile>(&path, &text)?.cpu),
            r if r.starts_with("chipset/") => db
                .chipsets
                .extend(from::<ChipsetFile>(&path, &text)?.chipset),
            _ => {}
        }
    }
    validate_required(&db)?;
    validate_duplicates(&db)?;
    validate_classes("pci", &db.pci_classes)?;
    validate_classes("usb", &db.usb_classes)?;
    validate_aliases(&db)?;
    sort_db(&mut db);
    Ok(db)
}
fn from<T: for<'a> Deserialize<'a>>(p: &Path, s: &str) -> Result<T, String> {
    toml::from_str(s).map_err(|e| format!("{}: {e}", p.display()))
}
fn sorted_toml_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    fn rec(v: &mut Vec<PathBuf>, p: &Path) -> Result<(), String> {
        for e in fs::read_dir(p).map_err(|e| format!("{}: {e}", p.display()))? {
            let p = e.map_err(|e| e.to_string())?.path();
            if p.is_dir() {
                rec(v, &p)?
            } else if p.extension().is_some_and(|e| e == "toml") {
                v.push(p)
            }
        }
        Ok(())
    }
    let mut v = Vec::new();
    rec(&mut v, root)?;
    v.sort();
    Ok(v)
}
fn validate_hex_widths(rel: &str, text: &str) -> Result<(), String> {
    for (n, l) in text.lines().enumerate() {
        let Some((k, r)) = l.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let val = r.trim().trim_end_matches(',');
        if let Some(hex) = val.strip_prefix("0x") {
            let w = match k {
                "vendor_id" | "device_id" | "product_id" | "id" => 4,
                "class" | "subclass" | "prog_if" => 2,
                _ => continue,
            };
            let got = hex.chars().take_while(|c| c.is_ascii_hexdigit()).count();
            if got != w || !hex.chars().take(w).all(|c| c.is_ascii_hexdigit()) {
                return Err(format!(
                    "{rel}:{}: `{k}` must be 0x followed by exactly {w} hex digits",
                    n + 1
                ));
            }
        }
    }
    Ok(())
}
fn empty(s: &str) -> bool {
    s.trim().is_empty()
}
fn validate_required(db: &Database) -> Result<(), String> {
    for c in &db.cpus {
        if empty(&c.vendor) || empty(&c.name) {
            return Err("cpu entries require vendor and name".into());
        }
    }
    for v in db.pci_vendors.iter().chain(db.usb_vendors.iter()) {
        if empty(&v.name) {
            return Err("vendor entries require name".into());
        }
    }
    for d in &db.pci_devices {
        if empty(&d.name) {
            return Err("pci device entries require name".into());
        }
    }
    for d in &db.usb_devices {
        if empty(&d.name) {
            return Err("usb device entries require name".into());
        }
    }
    for c in db.pci_classes.iter().chain(db.usb_classes.iter()) {
        if c.subclass.is_none() && c.prog_if.is_some() {
            return Err("class prog_if requires subclass".into());
        }
        if empty(&c.name) {
            return Err("class entries require name".into());
        }
    }
    for b in &db.block {
        if empty(&b.kind) || empty(&b.name) || empty(&b.driver_hint) {
            return Err("block entries require kind, name, and driver_hint".into());
        }
    }
    for s in db.input.iter().chain(db.char_devs.iter()) {
        if empty(&s.kind) || empty(&s.name) {
            return Err("simple device entries require kind and name".into());
        }
    }
    Ok(())
}
fn validate_duplicates(db: &Database) -> Result<(), String> {
    seen1("duplicate PCI vendor", db.pci_vendors.iter().map(|v| v.id))?;
    seen1("duplicate USB vendor", db.usb_vendors.iter().map(|v| v.id))?;
    seen1(
        "duplicate PCI device",
        db.pci_devices.iter().map(|d| (d.vendor_id, d.device_id)),
    )?;
    seen1(
        "duplicate USB device",
        db.usb_devices.iter().map(|d| (d.vendor_id, d.product_id)),
    )?;
    Ok(())
}
fn seen1<T: Ord + std::fmt::Debug>(msg: &str, it: impl Iterator<Item = T>) -> Result<(), String> {
    let mut s = BTreeSet::new();
    for x in it {
        if !s.insert(x) {
            return Err(format!("{msg}: {:?}", s.iter().next_back().unwrap()));
        }
    }
    Ok(())
}
fn validate_classes(kind: &str, cs: &[Class]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for c in cs {
        let key = (c.class, c.subclass, c.prog_if);
        if !seen.insert(key) {
            return Err(format!("duplicate {kind} class tuple {key:?}"));
        }
        if c.prog_if.is_some() && c.subclass.is_none() {
            return Err(format!(
                "{kind} class {:#04x} prog_if requires subclass",
                c.class
            ));
        }
    }
    Ok(())
}
fn validate_aliases(db: &Database) -> Result<(), String> {
    let cats: [&str; 6] = ["cpu", "pci", "usb", "block", "input", "char"];
    for (cat, map) in &db.aliases {
        if !cats.contains(&cat.as_str()) {
            return Err(format!("unknown alias category `{cat}`"));
        }
        let mut names = BTreeSet::new();
        for (a, t) in map {
            if empty(a) || empty(t) || !names.insert(a) {
                return Err(format!("invalid alias `{cat}.{a}`"));
            }
            match cat.as_str() {
                "pci" if !alias_pci(db, t) => {
                    return Err(format!("alias `{a}` targets missing PCI descriptor `{t}`"))
                }
                "usb" if !alias_usb(db, t) => {
                    return Err(format!("alias `{a}` targets missing USB descriptor `{t}`"))
                }
                "block"
                    if !db
                        .block
                        .iter()
                        .any(|b| t.as_str() == format!("block:{}", b.kind)) =>
                {
                    return Err(format!(
                        "alias `{a}` targets missing block descriptor `{t}`"
                    ))
                }
                "input"
                    if !db
                        .input
                        .iter()
                        .any(|x| t.as_str() == format!("input:{}", x.kind)) =>
                {
                    return Err(format!(
                        "alias `{a}` targets missing input descriptor `{t}`"
                    ))
                }
                "char"
                    if !db
                        .char_devs
                        .iter()
                        .any(|x| t.as_str() == format!("char:{}", x.kind)) =>
                {
                    return Err(format!("alias `{a}` targets missing char descriptor `{t}`"))
                }
                "cpu" if !alias_cpu(db, t) => {
                    return Err(format!("alias `{a}` targets missing CPU descriptor `{t}`"))
                }
                _ => {}
            }
        }
    }
    Ok(())
}
fn alias_pci(db: &Database, t: &str) -> bool {
    let p: Vec<_> = t.split(':').collect();
    p.len() == 3
        && p[0] == "pci"
        && u16hex(p[1]).zip(u16hex(p[2])).is_some_and(|(v, d)| {
            db.pci_devices
                .iter()
                .any(|x| x.vendor_id == v && x.device_id == d)
        })
}
fn alias_usb(db: &Database, t: &str) -> bool {
    let p: Vec<_> = t.split(':').collect();
    (p.len() == 3
        && p[0] == "usb"
        && u16hex(p[1]).zip(u16hex(p[2])).is_some_and(|(v, d)| {
            db.usb_devices
                .iter()
                .any(|x| x.vendor_id == v && x.product_id == d)
        }))
        || (p.len() == 2
            && p[0] == "usb-class"
            && u8hex(p[1]).is_some_and(|c| db.usb_classes.iter().any(|x| x.class == c)))
}
fn alias_cpu(db: &Database, t: &str) -> bool {
    let p: Vec<_> = t.split(':').collect();
    p.len() >= 3
        && p[0].starts_with("cpu/")
        && u16hex(p[2])
            .is_some_and(|fam| db.cpus.iter().any(|c| c.vendor == p[1] && c.family == fam))
}
fn u16hex(s: &str) -> Option<u16> {
    u16::from_str_radix(s.strip_prefix("0x")?, 16).ok()
}
fn u8hex(s: &str) -> Option<u8> {
    u8::from_str_radix(s.strip_prefix("0x")?, 16).ok()
}
fn sort_db(db: &mut Database) {
    db.cpus.sort_by_key(|c| {
        (
            c.vendor.clone(),
            c.family,
            c.model,
            c.stepping,
            c.name.clone(),
        )
    });
    db.pci_vendors.sort_by_key(|v| v.id);
    db.usb_vendors.sort_by_key(|v| v.id);
    db.pci_devices.sort_by_key(|d| (d.vendor_id, d.device_id));
    db.usb_devices.sort_by_key(|d| (d.vendor_id, d.product_id));
    db.pci_classes
        .sort_by_key(|c| (c.class, c.subclass, c.prog_if));
    db.usb_classes
        .sort_by_key(|c| (c.class, c.subclass, c.prog_if));
    db.block.sort_by_key(|b| b.kind.clone());
    db.input.sort_by_key(|s| s.kind.clone());
    db.char_devs.sort_by_key(|s| s.kind.clone());
    db.chipsets.sort_by_key(|c| c.family.clone());
}

pub fn write_generated(db: &Database, out: &Path) -> Result<(), String> {
    let s = generate(db);
    if let Some(p) = out.parent() {
        fs::create_dir_all(p).map_err(|e| e.to_string())?
    }
    fs::write(out, s).map_err(|e| e.to_string())
}
pub fn generate(db: &Database) -> String {
    let mut o = String::new();
    o.push_str("//! Generated by gen-device-db. Do not edit by hand.\n#![allow(dead_code)]\n\n");
    emit_simple(
        &mut o,
        "InputDeviceDescriptor",
        "INPUT_DEVICE_DESCRIPTORS",
        &db.input,
    );
    emit_block(&mut o, &db.block);
    emit_simple(
        &mut o,
        "CharDeviceDescriptor",
        "CHAR_DEVICE_DESCRIPTORS",
        &db.char_devs,
    );
    emit_vendors(
        &mut o,
        "PciVendorDescriptor",
        "PCI_VENDOR_DESCRIPTORS",
        &db.pci_vendors,
    );
    emit_pci_devices(&mut o, &db.pci_devices);
    emit_classes(
        &mut o,
        "PciClassDescriptor",
        "PCI_CLASS_DESCRIPTORS",
        &db.pci_classes,
    );
    emit_vendors(
        &mut o,
        "UsbVendorDescriptor",
        "USB_VENDOR_DESCRIPTORS",
        &db.usb_vendors,
    );
    emit_usb_devices(&mut o, &db.usb_devices);
    emit_classes(
        &mut o,
        "UsbClassDescriptor",
        "USB_CLASS_DESCRIPTORS",
        &db.usb_classes,
    );
    emit_chipsets(&mut o, &db.chipsets);
    emit_cpu(&mut o, db);
    o
}
fn q(s: &str) -> String {
    format!("{:?}", s)
}
fn hints(v: &[String]) -> String {
    format!(
        "&[{}]",
        v.iter().map(|x| q(x)).collect::<Vec<_>>().join(", ")
    )
}
fn emit_simple(o: &mut String, ty: &str, name: &str, v: &[Simple]) {
    o.push_str(&format!("#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub struct {ty} {{ pub kind: &'static str, pub name: &'static str, pub driver_hints: &'static [&'static str] }}\nconst {name}: &[{ty}] = &[\n"));
    for x in v {
        o.push_str(&format!(
            "    {ty} {{ kind: {}, name: {}, driver_hints: {} }},\n",
            q(&x.kind),
            q(&x.name),
            hints(&x.driver_hints)
        ));
    }
    o.push_str("];\n\n");
}
fn emit_block(o: &mut String, v: &[Block]) {
    o.push_str("#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub struct BlockDeviceDescriptor { pub kind: &'static str, pub name: &'static str, pub driver_hint: &'static str, pub default_block_size: u32 }\nconst BLOCK_DEVICE_DESCRIPTORS: &[BlockDeviceDescriptor] = &[\n");
    for x in v {
        o.push_str(&format!("    BlockDeviceDescriptor {{ kind: {}, name: {}, driver_hint: {}, default_block_size: {} }},\n",q(&x.kind),q(&x.name),q(&x.driver_hint),x.default_block_size));
    }
    o.push_str("];\n\n");
}
fn emit_vendors(o: &mut String, ty: &str, name: &str, v: &[Vendor]) {
    o.push_str(&format!(
        "#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub struct {ty} {{ pub id: u16, pub name: &'static str, pub aliases: &'static [&'static str] }}\nconst {name}: &[{ty}] = &[\n"
    ));
    for x in v {
        o.push_str(&format!(
            "    {ty} {{ id: {:#06x}, name: {}, aliases: {} }},\n",
            x.id,
            q(&x.name),
            hints(&x.aliases)
        ));
    }
    o.push_str("];\n\n");
}

fn emit_pci_devices(o: &mut String, v: &[PciDevice]) {
    o.push_str("#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub struct PciDeviceDescriptor { pub vendor_id: u16, pub device_id: u16, pub name: &'static str, pub driver_hint: Option<&'static str> }\nconst PCI_DEVICE_DESCRIPTORS: &[PciDeviceDescriptor] = &[\n");
    for x in v {
        o.push_str(&format!(
            "    PciDeviceDescriptor {{ vendor_id: {:#06x}, device_id: {:#06x}, name: {}, driver_hint: {} }},\n",
            x.vendor_id, x.device_id, q(&x.name), optstr(x.driver_hint.as_deref())
        ));
    }
    o.push_str("];\n\n");
}

fn emit_usb_devices(o: &mut String, v: &[UsbDevice]) {
    o.push_str("#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub struct UsbDeviceDescriptor { pub vendor_id: u16, pub product_id: u16, pub name: &'static str, pub driver_hint: Option<&'static str> }\nconst USB_DEVICE_DESCRIPTORS: &[UsbDeviceDescriptor] = &[\n");
    for x in v {
        o.push_str(&format!(
            "    UsbDeviceDescriptor {{ vendor_id: {:#06x}, product_id: {:#06x}, name: {}, driver_hint: {} }},\n",
            x.vendor_id, x.product_id, q(&x.name), optstr(x.driver_hint.as_deref())
        ));
    }
    o.push_str("];\n\n");
}

fn emit_classes(o: &mut String, ty: &str, name: &str, v: &[Class]) {
    o.push_str(&format!("#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub struct {ty} {{ pub class: u8, pub subclass: Option<u8>, pub prog_if: Option<u8>, pub name: &'static str, pub driver_hint: Option<&'static str> }}\nconst {name}: &[{ty}] = &[\n"));
    for x in v {
        o.push_str(&format!(
            "    {ty} {{ class: {:#04x}, subclass: {}, prog_if: {}, name: {}, driver_hint: {} }},\n",
            x.class, opt8(x.subclass), opt8(x.prog_if), q(&x.name), optstr(x.driver_hint.as_deref())
        ));
    }
    o.push_str("];\n\n");
}

fn emit_chipsets(o: &mut String, v: &[Chipset]) {
    o.push_str("#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub struct ChipsetDescriptor { pub family: &'static str, pub name: &'static str, pub components: &'static [&'static str], pub driver_hints: &'static [&'static str] }\nconst CHIPSET_DESCRIPTORS: &[ChipsetDescriptor] = &[\n");
    for x in v {
        o.push_str(&format!(
            "    ChipsetDescriptor {{ family: {}, name: {}, components: {}, driver_hints: {} }},\n",
            q(&x.family),
            q(&x.name),
            hints(&x.components),
            hints(&x.driver_hints)
        ));
    }
    o.push_str("];\n\n");
}

fn optstr(v: Option<&str>) -> String {
    v.map(|s| format!("Some({})", q(s)))
        .unwrap_or("None".into())
}

fn emit_cpu(o: &mut String, db: &Database) {
    o.push_str("#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub struct CpuInfo { pub family: u16, pub model: Option<u16>, pub stepping: Option<u8>, pub name: &'static str, pub codename: Option<&'static str>, pub driver_hints: &'static [&'static str], pub diagnostic_name: &'static str }\n");
    for (vendor, constn) in [
        ("AuthenticAMD", "AMD_CPU_INFOS"),
        ("GenuineIntel", "INTEL_CPU_INFOS"),
    ] {
        o.push_str(&format!("const {constn}: &[CpuInfo] = &[\n"));
        for c in db.cpus.iter().filter(|c| c.vendor == vendor) {
            let diag = format!(
                "{}{}{}",
                c.name,
                c.codename
                    .as_ref()
                    .map(|x| format!(" ({x})"))
                    .unwrap_or_default(),
                if c.driver_hints.is_empty() {
                    String::new()
                } else {
                    format!("; hints: {}", c.driver_hints.join(", "))
                }
            );
            o.push_str(&format!("    CpuInfo {{ family: {:#04x}, model: {}, stepping: {}, name: {}, codename: {}, driver_hints: {}, diagnostic_name: {} }},\n",c.family,opt16(c.model),opt8(c.stepping),q(&c.name),c.codename.as_ref().map(|s|format!("Some({})",q(s))).unwrap_or("None".into()),hints(&c.driver_hints),q(&diag)));
        }
        o.push_str("];\n");
    }
    o.push_str("const fn lookup_cpu(table: &'static [CpuInfo], family: u16, model: u16, stepping: u8) -> Option<&'static CpuInfo> { let mut mf=None; let mut ff=None; let mut i=0; while i<table.len(){ let e=&table[i]; if e.family==family { match (e.model,e.stepping){ (Some(m),Some(s)) if m==model && s==stepping => return Some(e), (Some(m),None) if m==model && mf.is_none()=>mf=Some(e), (None,None) if ff.is_none()=>ff=Some(e), _=>{} } } i+=1;} match mf { Some(e)=>Some(e), None=>ff } }\npub const fn lookup_cpu_amd(family:u16, model:u16, stepping:u8)->Option<&'static CpuInfo>{lookup_cpu(AMD_CPU_INFOS,family,model,stepping)}\npub const fn lookup_cpu_intel(family:u16, model:u16, stepping:u8)->Option<&'static CpuInfo>{lookup_cpu(INTEL_CPU_INFOS,family,model,stepping)}\npub fn lookup_block_kind(kind:&str)->Option<&'static BlockDeviceDescriptor>{BLOCK_DEVICE_DESCRIPTORS.iter().find(|e|e.kind==kind)}\npub fn lookup_input_kind(kind:&str)->Option<&'static InputDeviceDescriptor>{INPUT_DEVICE_DESCRIPTORS.iter().find(|e|e.kind==kind)}\n");
}
fn opt16(v: Option<u16>) -> String {
    v.map(|x| format!("Some({:#04x})", x))
        .unwrap_or("None".into())
}
fn opt8(v: Option<u8>) -> String {
    v.map(|x| format!("Some({:#04x})", x))
        .unwrap_or("None".into())
}
