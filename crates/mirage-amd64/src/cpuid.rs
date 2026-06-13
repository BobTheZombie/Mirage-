//! AMD CPUID parsing primitives.
//!
//! The parser is deliberately data-driven: tests and early supervisor mocks can
//! inject CPUID leaves without executing hardware instructions. Hardware reads
//! are only compiled for explicit `hw-amd64` x86_64 builds.

/// Raw CPUID register values for one `(leaf, subleaf)` query.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CpuidLeaf {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

impl CpuidLeaf {
    pub const fn new(eax: u32, ebx: u32, ecx: u32, edx: u32) -> Self {
        Self { eax, ebx, ecx, edx }
    }
}

/// Injectable CPUID source used by parsers and tests.
pub trait AmdCpuidReader {
    fn cpuid(&self, leaf: u32, subleaf: u32) -> CpuidLeaf;
}

/// Hardware-backed CPUID reader.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HardwareCpuid;

impl AmdCpuidReader for HardwareCpuid {
    fn cpuid(&self, leaf: u32, subleaf: u32) -> CpuidLeaf {
        backend_cpuid(leaf, subleaf)
    }
}

/// CPU vendor decoded from CPUID leaf `0x0000_0000`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum AmdVendor {
    Amd,
    Other([u8; 12]),
}

impl AmdVendor {
    pub const AUTHENTIC_AMD: [u8; 12] = *b"AuthenticAMD";

    pub const fn from_bytes(bytes: [u8; 12]) -> Self {
        if bytes_eq(bytes, Self::AUTHENTIC_AMD) {
            Self::Amd
        } else {
            Self::Other(bytes)
        }
    }

    pub const fn as_bytes(self) -> [u8; 12] {
        match self {
            Self::Amd => Self::AUTHENTIC_AMD,
            Self::Other(bytes) => bytes,
        }
    }
}

const fn bytes_eq(left: [u8; 12], right: [u8; 12]) -> bool {
    let mut index = 0;
    while index < 12 {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

/// AMD CPU family value after applying extended-family decode rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdCpuFamily(pub u16);

/// AMD CPU model value after applying extended-model decode rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdCpuModel(pub u16);

/// AMD CPU stepping value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdCpuStepping(pub u8);

/// Feature bits surfaced by AMD CPUID leaves.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AmdFeatureSet {
    pub apic: bool,
    pub x2apic: bool,
    pub tsc: bool,
    pub constant_tsc: bool,
    pub nonstop_tsc: bool,
    pub invariant_tsc: bool,
    pub sse: bool,
    pub sse2: bool,
    pub sse3: bool,
    pub ssse3: bool,
    pub sse4_1: bool,
    pub sse4_2: bool,
    pub avx: bool,
    pub avx2: bool,
    pub smep: bool,
    pub smap: bool,
    pub pat: bool,
    pub mtrr: bool,
    pub nx: bool,
    pub syscall_sysret: bool,
    pub rdtscp: bool,
    pub svm: bool,
    pub sme: bool,
    /// True only if an explicitly modeled, reported CPUID feature bit says so.
    /// Current AMD64 CPUID parsing in Mirage does not infer IOMMU presence from
    /// vendor, SVM, PCI/ACPI tables, or platform presence.
    pub iommu: bool,
}

/// Snapshot of the CPUID leaves Mirage currently parses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AmdCpuId {
    max_basic_leaf: u32,
    max_extended_leaf: u32,
    leaf0: CpuidLeaf,
    leaf1: CpuidLeaf,
    leaf7_0: CpuidLeaf,
    ext_80000001: CpuidLeaf,
    ext_80000005: CpuidLeaf,
    ext_80000006: CpuidLeaf,
    ext_80000007: CpuidLeaf,
    ext_80000008: CpuidLeaf,
    ext_8000000a: CpuidLeaf,
    brand_80000002: CpuidLeaf,
    brand_80000003: CpuidLeaf,
    brand_80000004: CpuidLeaf,
    ext_8000001e: CpuidLeaf,
    ext_8000001f: CpuidLeaf,
}

impl AmdCpuId {
    /// Read CPUID leaves from the hardware backend when enabled, otherwise
    /// return an empty snapshot suitable for non-hardware builds.
    pub fn read() -> Self {
        Self::from_reader(&HardwareCpuid)
    }

    /// Build an AMD CPUID snapshot from an injectable reader.
    pub fn from_reader(reader: &impl AmdCpuidReader) -> Self {
        let leaf0 = reader.cpuid(0x0000_0000, 0);
        let max_basic_leaf = leaf0.eax;
        let ext0 = reader.cpuid(0x8000_0000, 0);
        let max_extended_leaf = ext0.eax;

        Self {
            max_basic_leaf,
            max_extended_leaf,
            leaf0,
            leaf1: read_if(reader, max_basic_leaf, 0x0000_0001, 0),
            leaf7_0: read_if(reader, max_basic_leaf, 0x0000_0007, 0),
            ext_80000001: read_if(reader, max_extended_leaf, 0x8000_0001, 0),
            ext_80000005: read_if(reader, max_extended_leaf, 0x8000_0005, 0),
            ext_80000006: read_if(reader, max_extended_leaf, 0x8000_0006, 0),
            ext_80000007: read_if(reader, max_extended_leaf, 0x8000_0007, 0),
            ext_80000008: read_if(reader, max_extended_leaf, 0x8000_0008, 0),
            ext_8000000a: read_if(reader, max_extended_leaf, 0x8000_000a, 0),
            brand_80000002: read_if(reader, max_extended_leaf, 0x8000_0002, 0),
            brand_80000003: read_if(reader, max_extended_leaf, 0x8000_0003, 0),
            brand_80000004: read_if(reader, max_extended_leaf, 0x8000_0004, 0),
            ext_8000001e: read_if(reader, max_extended_leaf, 0x8000_001e, 0),
            ext_8000001f: read_if(reader, max_extended_leaf, 0x8000_001f, 0),
        }
    }

    pub const fn max_basic_leaf(self) -> u32 {
        self.max_basic_leaf
    }

    pub const fn max_extended_leaf(self) -> u32 {
        self.max_extended_leaf
    }

    pub const fn leaf(self, leaf: u32) -> CpuidLeaf {
        match leaf {
            0x0000_0000 => self.leaf0,
            0x0000_0001 => self.leaf1,
            0x0000_0007 => self.leaf7_0,
            0x8000_0001 => self.ext_80000001,
            0x8000_0005 => self.ext_80000005,
            0x8000_0006 => self.ext_80000006,
            0x8000_0007 => self.ext_80000007,
            0x8000_0008 => self.ext_80000008,
            0x8000_000a => self.ext_8000000a,
            0x8000_0002 => self.brand_80000002,
            0x8000_0003 => self.brand_80000003,
            0x8000_0004 => self.brand_80000004,
            0x8000_001e => self.ext_8000001e,
            0x8000_001f => self.ext_8000001f,
            _ => CpuidLeaf::new(0, 0, 0, 0),
        }
    }

    /// Decode the vendor string from CPUID `EBX:EDX:ECX` order.
    pub fn vendor(self) -> AmdVendor {
        let mut bytes = [0u8; 12];
        bytes[0..4].copy_from_slice(&self.leaf0.ebx.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.leaf0.edx.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.leaf0.ecx.to_le_bytes());
        AmdVendor::from_bytes(bytes)
    }

    /// Decode the 48-byte processor brand string from extended CPUID leaves.
    pub fn brand_string(self) -> [u8; 48] {
        let mut bytes = [0u8; 48];
        let leaves = [
            self.brand_80000002,
            self.brand_80000003,
            self.brand_80000004,
        ];
        let mut out = 0usize;
        let mut index = 0usize;
        while index < leaves.len() {
            let leaf = leaves[index];
            for reg in [leaf.eax, leaf.ebx, leaf.ecx, leaf.edx] {
                bytes[out..out + 4].copy_from_slice(&reg.to_le_bytes());
                out += 4;
            }
            index += 1;
        }
        bytes
    }

    /// Decode AMD family/model/stepping from CPUID leaf `0x0000_0001:EAX`.
    pub const fn family_model_stepping(self) -> (AmdCpuFamily, AmdCpuModel, AmdCpuStepping) {
        let eax = self.leaf1.eax;
        let stepping = (eax & 0x0f) as u8;
        let base_model = ((eax >> 4) & 0x0f) as u16;
        let base_family = ((eax >> 8) & 0x0f) as u16;
        let ext_model = ((eax >> 16) & 0x0f) as u16;
        let ext_family = ((eax >> 20) & 0xff) as u16;

        let family = if base_family == 0x0f {
            base_family + ext_family
        } else {
            base_family
        };
        let model = if base_family == 0x0f {
            base_model + (ext_model << 4)
        } else {
            base_model
        };

        (
            AmdCpuFamily(family),
            AmdCpuModel(model),
            AmdCpuStepping(stepping),
        )
    }

    /// Decode features only from explicit reported CPUID feature bits.
    pub const fn features(self) -> AmdFeatureSet {
        let invariant_tsc = bit(self.ext_80000007.edx, 8);

        AmdFeatureSet {
            apic: bit(self.leaf1.edx, 9),
            x2apic: bit(self.leaf1.ecx, 21),
            tsc: bit(self.leaf1.edx, 4),
            // AMD exposes invariant TSC through Fn8000_0007:EDX[8]. Mirage
            // treats the constant/nonstop helpers as aliases of that reported
            // architectural bit, never as vendor/model inferences.
            constant_tsc: invariant_tsc,
            nonstop_tsc: invariant_tsc,
            invariant_tsc,
            sse: bit(self.leaf1.edx, 25),
            sse2: bit(self.leaf1.edx, 26),
            sse3: bit(self.leaf1.ecx, 0),
            ssse3: bit(self.leaf1.ecx, 9),
            sse4_1: bit(self.leaf1.ecx, 19),
            sse4_2: bit(self.leaf1.ecx, 20),
            avx: bit(self.leaf1.ecx, 28),
            avx2: bit(self.leaf7_0.ebx, 5),
            smep: bit(self.leaf7_0.ebx, 7),
            smap: bit(self.leaf7_0.ebx, 20),
            pat: bit(self.leaf1.edx, 16),
            mtrr: bit(self.leaf1.edx, 12),
            nx: bit(self.ext_80000001.edx, 20),
            syscall_sysret: bit(self.ext_80000001.edx, 11),
            rdtscp: bit(self.ext_80000001.edx, 27),
            svm: bit(self.ext_80000001.ecx, 2),
            sme: bit(self.ext_8000001f.eax, 0),
            iommu: false,
        }
    }
}

const fn bit(value: u32, index: u32) -> bool {
    (value & (1 << index)) != 0
}

fn read_if(reader: &impl AmdCpuidReader, max_leaf: u32, leaf: u32, subleaf: u32) -> CpuidLeaf {
    if max_leaf >= leaf {
        reader.cpuid(leaf, subleaf)
    } else {
        CpuidLeaf::new(0, 0, 0, 0)
    }
}

#[cfg(all(feature = "hw-amd64", target_arch = "x86_64"))]
fn backend_cpuid(leaf: u32, subleaf: u32) -> CpuidLeaf {
    use core::arch::x86_64::__cpuid_count;

    let result = __cpuid_count(leaf, subleaf);
    CpuidLeaf::new(result.eax, result.ebx, result.ecx, result.edx)
}

#[cfg(any(not(feature = "hw-amd64"), not(target_arch = "x86_64")))]
fn backend_cpuid(_leaf: u32, _subleaf: u32) -> CpuidLeaf {
    CpuidLeaf::new(0, 0, 0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct MockCpuid<'a> {
        leaves: &'a [(u32, u32, CpuidLeaf)],
    }

    impl AmdCpuidReader for MockCpuid<'_> {
        fn cpuid(&self, leaf: u32, subleaf: u32) -> CpuidLeaf {
            let mut index = 0;
            while index < self.leaves.len() {
                let (mock_leaf, mock_subleaf, value) = self.leaves[index];
                if mock_leaf == leaf && mock_subleaf == subleaf {
                    return value;
                }
                index += 1;
            }
            CpuidLeaf::new(0, 0, 0, 0)
        }
    }

    const fn vendor_leaf() -> CpuidLeaf {
        CpuidLeaf::new(
            0x0000_0007,
            u32::from_le_bytes(*b"Auth"),
            u32::from_le_bytes(*b"cAMD"),
            u32::from_le_bytes(*b"enti"),
        )
    }

    #[test]
    fn mock_cpuid_leaf_parsing_decodes_vendor_order() {
        let cpu = AmdCpuId::from_reader(&MockCpuid {
            leaves: &[
                (0x0000_0000, 0, vendor_leaf()),
                (0x8000_0000, 0, CpuidLeaf::new(0x8000_0001, 0, 0, 0)),
            ],
        });

        assert_eq!(cpu.vendor(), AmdVendor::Amd);
        assert_eq!(cpu.max_basic_leaf(), 0x0000_0007);
        assert_eq!(cpu.max_extended_leaf(), 0x8000_0001);
    }

    #[test]
    fn amd_family_model_stepping_decode_applies_extended_fields() {
        let eax = 0x0080_0f82; // base family f + ext family 8 = 0x17, model 0x08, stepping 2.
        let cpu = AmdCpuId::from_reader(&MockCpuid {
            leaves: &[
                (0x0000_0000, 0, vendor_leaf()),
                (0x0000_0001, 0, CpuidLeaf::new(eax, 0, 0, 0)),
                (0x8000_0000, 0, CpuidLeaf::new(0x8000_0001, 0, 0, 0)),
            ],
        });

        assert_eq!(
            cpu.family_model_stepping(),
            (AmdCpuFamily(0x17), AmdCpuModel(0x08), AmdCpuStepping(2))
        );
    }

    #[test]
    fn feature_decode_uses_reported_bits_only() {
        let leaf1_ecx = (1 << 0) | (1 << 9) | (1 << 19) | (1 << 20) | (1 << 21) | (1 << 28);
        let leaf1_edx = (1 << 4) | (1 << 9) | (1 << 12) | (1 << 16) | (1 << 25) | (1 << 26);
        let leaf7_ebx = (1 << 5) | (1 << 7) | (1 << 20);
        let ext1_ecx = 1 << 2;
        let ext1_edx = (1 << 11) | (1 << 20) | (1 << 27);
        let ext7_edx = 1 << 8;
        let cpu = AmdCpuId::from_reader(&MockCpuid {
            leaves: &[
                (0x0000_0000, 0, vendor_leaf()),
                (0x0000_0001, 0, CpuidLeaf::new(0, 0, leaf1_ecx, leaf1_edx)),
                (0x0000_0007, 0, CpuidLeaf::new(0, leaf7_ebx, 0, 0)),
                (0x8000_0000, 0, CpuidLeaf::new(0x8000_001f, 0, 0, 0)),
                (0x8000_0001, 0, CpuidLeaf::new(0, 0, ext1_ecx, ext1_edx)),
                (0x8000_0007, 0, CpuidLeaf::new(0, 0, 0, ext7_edx)),
                (0x8000_000a, 0, CpuidLeaf::new(0, 0, 0, u32::MAX)),
                (0x8000_001f, 0, CpuidLeaf::new(1, 0, 0, 0)),
            ],
        });

        let features = cpu.features();
        assert!(features.apic);
        assert!(features.x2apic);
        assert!(features.tsc);
        assert!(features.constant_tsc);
        assert!(features.nonstop_tsc);
        assert!(features.invariant_tsc);
        assert!(features.sse);
        assert!(features.sse2);
        assert!(features.sse3);
        assert!(features.ssse3);
        assert!(features.sse4_1);
        assert!(features.sse4_2);
        assert!(features.avx);
        assert!(features.avx2);
        assert!(features.smep);
        assert!(features.smap);
        assert!(features.pat);
        assert!(features.mtrr);
        assert!(features.nx);
        assert!(features.syscall_sysret);
        assert!(features.rdtscp);
        assert!(features.svm);
        assert!(features.sme);
        assert!(!features.iommu);
    }

    #[test]
    fn brand_string_decodes_extended_brand_leaves() {
        let cpu = AmdCpuId::from_reader(&MockCpuid {
            leaves: &[
                (0x0000_0000, 0, vendor_leaf()),
                (0x8000_0000, 0, CpuidLeaf::new(0x8000_0004, 0, 0, 0)),
                (
                    0x8000_0002,
                    0,
                    CpuidLeaf::new(
                        u32::from_le_bytes(*b"AMD "),
                        u32::from_le_bytes(*b"Ryze"),
                        u32::from_le_bytes(*b"n 5 "),
                        u32::from_le_bytes(*b"4500"),
                    ),
                ),
                (
                    0x8000_0003,
                    0,
                    CpuidLeaf::new(
                        u32::from_le_bytes(*b"U wi"),
                        u32::from_le_bytes(*b"th R"),
                        u32::from_le_bytes(*b"adeo"),
                        u32::from_le_bytes(*b"n Gr"),
                    ),
                ),
                (
                    0x8000_0004,
                    0,
                    CpuidLeaf::new(
                        u32::from_le_bytes(*b"aphi"),
                        u32::from_le_bytes(*b"cs  "),
                        0,
                        0,
                    ),
                ),
            ],
        });

        assert!(cpu.brand_string().starts_with(b"AMD Ryzen 5 4500U"));
    }

    #[test]
    fn unavailable_leaf_does_not_enable_features_from_presence() {
        let cpu = AmdCpuId::from_reader(&MockCpuid {
            leaves: &[
                (0x0000_0000, 0, vendor_leaf()),
                (0x8000_0000, 0, CpuidLeaf::new(0x8000_0000, 0, 0, 0)),
            ],
        });

        assert_eq!(cpu.features(), AmdFeatureSet::default());
    }
}
