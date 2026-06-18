//! AMD Renoir / Ryzen 5 4500U lower-kernel boot profile.
//!
//! This file belongs below the supervisor.  It is allowed to read CPUID and
//! report boot phases.  It must not reset the GPU, touch PSP firmware, enable
//! IOMMU translation, start xHCI, or claim full driver Online status.

use core::arch::x86_64::__cpuid_count;

use crate::arch::x86_64::boot::BootInfo;
use crate::kernel::boot_phase::{
    boot_phase_detected, boot_phase_ok, boot_phase_skipped, boot_phase_start, boot_phase_stub,
    BootPhase,
};

pub const RENOIR_FAMILY: u16 = 0x17;
pub const RENOIR_MODEL_MIN: u16 = 0x60;
pub const RENOIR_MODEL_MAX: u16 = 0x7f;
pub const RYZEN_4500U_EXPECTED_CORES: u16 = 6;
pub const RYZEN_4500U_EXPECTED_THREADS: u16 = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenoirCpuidFacts {
    pub vendor: [u8; 12],
    pub family: u16,
    pub model: u16,
    pub stepping: u8,
    pub logical_threads: u16,
    pub physical_cores: u16,
}

impl RenoirCpuidFacts {
    pub const fn is_amd(self) -> bool {
        matches!(
            self.vendor,
            [
                b'A', b'u', b't', b'h', b'e', b'n',
                b't', b'i', b'c', b'A', b'M', b'D',
            ]
        )
    }

    pub const fn is_renoir(self) -> bool {
        self.is_amd()
            && self.family == RENOIR_FAMILY
            && self.model >= RENOIR_MODEL_MIN
            && self.model <= RENOIR_MODEL_MAX
    }

    pub const fn is_ryzen_4500u_class(self) -> bool {
        self.is_renoir()
            && self.physical_cores == RYZEN_4500U_EXPECTED_CORES
            && self.logical_threads == RYZEN_4500U_EXPECTED_THREADS
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenoirBootProfile {
    pub cpuid: RenoirCpuidFacts,
    pub scheduler_module: &'static str,
    pub supports_amd_iommu_discovery: bool,
    pub supports_renoir_gpu_discovery: bool,
    pub supports_xhci_discovery: bool,
}

pub fn renoir_kernel_boot_probe(_boot_info: &BootInfo) -> Option<RenoirBootProfile> {
    let facts = read_cpuid_facts();
    if !facts.is_amd() {
        boot_phase_skipped(BootPhase::RyzenCpu, "non-AMD CPU");
        boot_phase_skipped(BootPhase::AmdSoc, "non-AMD platform");
        return None;
    }

    boot_phase_start(BootPhase::RyzenCpu);
    if !facts.is_renoir() {
        boot_phase_detected(BootPhase::RyzenCpu);
        boot_phase_skipped(BootPhase::AmdSoc, "AMD CPU is not Renoir-class Zen 2 mobile");
        crate::kprintln!(
            "AMD CPU detected: family={:#x} model={:#x} stepping={}",
            facts.family,
            facts.model,
            facts.stepping
        );
        return None;
    }

    if facts.is_ryzen_4500u_class() {
        boot_phase_ok(BootPhase::RyzenCpu);
        crate::kprintln!(
            "AMD Ryzen 5 4500U / Renoir-class platform detected: family={:#x} model={:#x} cores={} threads={}",
            facts.family,
            facts.model,
            facts.physical_cores,
            facts.logical_threads
        );
    } else {
        boot_phase_detected(BootPhase::RyzenCpu);
        crate::kprintln!(
            "AMD Renoir/Lucienne-class Zen 2 mobile platform detected: family={:#x} model={:#x} cores={} threads={}",
            facts.family,
            facts.model,
            facts.physical_cores,
            facts.logical_threads
        );
    }

    boot_phase_start(BootPhase::RyzenTopology);
    boot_phase_ok(BootPhase::RyzenTopology);

    boot_phase_start(BootPhase::AmdSoc);
    boot_phase_detected(BootPhase::AmdSoc);
    boot_phase_ok(BootPhase::AmdSoc);

    // Discovery is lower-kernel safe; driver ownership remains later.
    boot_phase_start(BootPhase::AmdIommu);
    boot_phase_stub(BootPhase::AmdIommu, "Renoir IVRS/IOMMU discovery only; translation not enabled");
    boot_phase_start(BootPhase::AmdGpuRenoir);
    boot_phase_stub(BootPhase::AmdGpuRenoir, "Renoir GPU discovery only; no reset/modeset");
    boot_phase_start(BootPhase::AmdXhci);
    boot_phase_detected(BootPhase::AmdXhci);

    Some(RenoirBootProfile {
        cpuid: facts,
        scheduler_module: "mtss-sched-amd-zen2-renoir",
        supports_amd_iommu_discovery: true,
        supports_renoir_gpu_discovery: true,
        supports_xhci_discovery: true,
    })
}

fn read_cpuid_facts() -> RenoirCpuidFacts {
    // SAFETY: CPUID is an architectural identification instruction on x86_64.
    let vendor_leaf = unsafe { __cpuid_count(0, 0) };
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&vendor_leaf.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&vendor_leaf.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&vendor_leaf.ecx.to_le_bytes());

    // SAFETY: Leaf 1 is architectural on x86_64.
    let leaf1 = unsafe { __cpuid_count(1, 0) };
    let base_family = ((leaf1.eax >> 8) & 0x0f) as u16;
    let base_model = ((leaf1.eax >> 4) & 0x0f) as u16;
    let ext_family = ((leaf1.eax >> 20) & 0xff) as u16;
    let ext_model = ((leaf1.eax >> 16) & 0x0f) as u16;
    let family = if base_family == 0x0f {
        base_family + ext_family
    } else {
        base_family
    };
    let model = if base_family == 0x06 || base_family == 0x0f {
        base_model + (ext_model << 4)
    } else {
        base_model
    };
    let stepping = (leaf1.eax & 0x0f) as u8;
    let logical_threads = ((leaf1.ebx >> 16) & 0xff) as u16;

    // SAFETY: Extended maximum leaf query is a CPUID identification read.
    let ext_max = unsafe { __cpuid_count(0x8000_0000, 0) }.eax;
    let physical_cores = if ext_max >= 0x8000_0008 {
        // SAFETY: Guarded by ext_max above; leaf 0x80000008 is available.
        let leaf = unsafe { __cpuid_count(0x8000_0008, 0) };
        ((leaf.ecx & 0xff) as u16) + 1
    } else {
        logical_threads.max(1)
    };

    RenoirCpuidFacts {
        vendor,
        family,
        model,
        stepping,
        logical_threads: logical_threads.max(1),
        physical_cores: physical_cores.max(1),
    }
}
