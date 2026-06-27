//! Generated no_std CPU diagnostic lookup tables.
//!
//! Source descriptors live under `devices/db/cpu/*.toml`. These tables are
//! diagnostic metadata only; Mirage still binds hardware through raw CPUID/PCI
//! facts, capabilities, and supervisor policy.

/// Known CPU metadata for platform diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuInfo {
    pub family: u16,
    pub model: Option<u16>,
    pub stepping: Option<u8>,
    pub name: &'static str,
    pub codename: Option<&'static str>,
    pub driver_hints: &'static [&'static str],
    pub diagnostic_name: &'static str,
}

const AMD_CPU_INFOS: &[CpuInfo] = &[
    CpuInfo {
        family: 0x17,
        model: Some(0x60),
        stepping: Some(0x1),
        name: "AMD Ryzen 5 4500U",
        codename: Some("Renoir"),
        driver_hints: &["amd/renoir", "amdgpu/displayd", "xhci/usbd"],
        diagnostic_name:
            "AMD Ryzen 5 4500U (Renoir; hints: amd/renoir, amdgpu/displayd, xhci/usbd)",
    },
    CpuInfo {
        family: 0x17,
        model: Some(0x60),
        stepping: None,
        name: "AMD Ryzen 4000 Mobile APU",
        codename: Some("Renoir"),
        driver_hints: &["amd/renoir", "amdgpu/displayd", "xhci/usbd"],
        diagnostic_name:
            "AMD Ryzen 4000 Mobile APU (Renoir; hints: amd/renoir, amdgpu/displayd, xhci/usbd)",
    },
    CpuInfo {
        family: 0x17,
        model: None,
        stepping: None,
        name: "AMD Zen/Zen+ CPU",
        codename: Some("Zen family 17h"),
        driver_hints: &["amd64", "ryzen"],
        diagnostic_name: "AMD Zen/Zen+ CPU (Zen family 17h; hints: amd64, ryzen)",
    },
    CpuInfo {
        family: 0x19,
        model: None,
        stepping: None,
        name: "AMD Zen 3/Zen 4 CPU",
        codename: Some("Zen family 19h"),
        driver_hints: &["amd64", "ryzen"],
        diagnostic_name: "AMD Zen 3/Zen 4 CPU (Zen family 19h; hints: amd64, ryzen)",
    },
];

const INTEL_CPU_INFOS: &[CpuInfo] = &[
    CpuInfo {
        family: 0x06,
        model: Some(0x3c),
        stepping: None,
        name: "Intel Core 4th Gen CPU",
        codename: Some("Haswell"),
        driver_hints: &["intel64"],
        diagnostic_name: "Intel Core 4th Gen CPU (Haswell; hints: intel64)",
    },
    CpuInfo {
        family: 0x06,
        model: Some(0x4e),
        stepping: None,
        name: "Intel Core 6th Gen Mobile CPU",
        codename: Some("Skylake-U/Y"),
        driver_hints: &["intel64"],
        diagnostic_name: "Intel Core 6th Gen Mobile CPU (Skylake-U/Y; hints: intel64)",
    },
    CpuInfo {
        family: 0x06,
        model: Some(0x55),
        stepping: None,
        name: "Intel Xeon Scalable CPU",
        codename: Some("Skylake-SP/Cascade Lake-SP"),
        driver_hints: &["intel64"],
        diagnostic_name: "Intel Xeon Scalable CPU (Skylake-SP/Cascade Lake-SP; hints: intel64)",
    },
    CpuInfo {
        family: 0x06,
        model: Some(0x6a),
        stepping: None,
        name: "Intel Xeon Scalable CPU",
        codename: Some("Ice Lake-SP"),
        driver_hints: &["intel64"],
        diagnostic_name: "Intel Xeon Scalable CPU (Ice Lake-SP; hints: intel64)",
    },
    CpuInfo {
        family: 0x06,
        model: None,
        stepping: None,
        name: "Intel 64 CPU",
        codename: Some("Family 6"),
        driver_hints: &["intel64"],
        diagnostic_name: "Intel 64 CPU (Family 6; hints: intel64)",
    },
];

const fn lookup_cpu(
    table: &'static [CpuInfo],
    family: u16,
    model: u16,
    stepping: u8,
) -> Option<&'static CpuInfo> {
    let mut model_fallback = None;
    let mut family_fallback = None;
    let mut index = 0;
    while index < table.len() {
        let entry = &table[index];
        if entry.family == family {
            match (entry.model, entry.stepping) {
                (Some(entry_model), Some(entry_stepping))
                    if entry_model == model && entry_stepping == stepping =>
                {
                    return Some(entry)
                }
                (Some(entry_model), None) if entry_model == model && model_fallback.is_none() => {
                    model_fallback = Some(entry)
                }
                (None, None) if family_fallback.is_none() => family_fallback = Some(entry),
                _ => {}
            }
        }
        index += 1;
    }
    match model_fallback {
        Some(entry) => Some(entry),
        None => family_fallback,
    }
}

pub const fn lookup_cpu_amd(family: u16, model: u16, stepping: u8) -> Option<&'static CpuInfo> {
    lookup_cpu(AMD_CPU_INFOS, family, model, stepping)
}

pub const fn lookup_cpu_intel(family: u16, model: u16, stepping: u8) -> Option<&'static CpuInfo> {
    lookup_cpu(INTEL_CPU_INFOS, family, model, stepping)
}
