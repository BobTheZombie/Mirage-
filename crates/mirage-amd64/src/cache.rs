//! AMD cache geometry parsing from CPUID leaves.

use crate::cpuid::{AmdCpuId, AmdCpuidReader};

/// Cache geometry reported by AMD extended CPUID leaves.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AmdCacheInfo {
    pub l1_data_size_kib: u16,
    pub l1_instruction_size_kib: u16,
    pub l1_data_line_size: u16,
    pub l1_instruction_line_size: u16,
    pub l2_size_kib: u16,
    pub l2_line_size: u16,
    pub l3_size_kib: u32,
    pub l3_line_size: u16,
}

impl AmdCacheInfo {
    /// Discover cache information from hardware CPUID when enabled.
    pub fn discover() -> Self {
        Self::from_cpuid(AmdCpuId::read())
    }

    /// Discover cache information from an injectable CPUID reader.
    pub fn discover_from(reader: &impl AmdCpuidReader) -> Self {
        Self::from_cpuid(AmdCpuId::from_reader(reader))
    }

    pub const fn from_cpuid(cpuid: AmdCpuId) -> Self {
        let l1 = cpuid.leaf(0x8000_0005);
        let l2_l3 = cpuid.leaf(0x8000_0006);

        Self {
            l1_data_size_kib: ((l1.ecx >> 24) & 0xff) as u16,
            l1_instruction_size_kib: ((l1.edx >> 24) & 0xff) as u16,
            l1_data_line_size: (l1.ecx & 0xff) as u16,
            l1_instruction_line_size: (l1.edx & 0xff) as u16,
            l2_size_kib: ((l2_l3.ecx >> 16) & 0xffff) as u16,
            l2_line_size: (l2_l3.ecx & 0xff) as u16,
            l3_size_kib: ((l2_l3.edx >> 18) & 0x3fff) * 512,
            l3_line_size: (l2_l3.edx & 0xff) as u16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpuid::{AmdCpuidReader, CpuidLeaf};

    struct MockCpuid;

    impl AmdCpuidReader for MockCpuid {
        fn cpuid(&self, leaf: u32, _subleaf: u32) -> CpuidLeaf {
            match leaf {
                0x0000_0000 => CpuidLeaf::new(0x0000_0000, 0, 0, 0),
                0x8000_0000 => CpuidLeaf::new(0x8000_0006, 0, 0, 0),
                0x8000_0005 => CpuidLeaf::new(0, 0, (32 << 24) | 64, (32 << 24) | 64),
                0x8000_0006 => CpuidLeaf::new(0, 0, (512 << 16) | 64, (16 << 18) | 64),
                _ => CpuidLeaf::new(0, 0, 0, 0),
            }
        }
    }

    #[test]
    fn cache_parser_decodes_amd_extended_cache_leaves() {
        let cache = AmdCacheInfo::discover_from(&MockCpuid);

        assert_eq!(cache.l1_data_size_kib, 32);
        assert_eq!(cache.l1_instruction_size_kib, 32);
        assert_eq!(cache.l1_data_line_size, 64);
        assert_eq!(cache.l1_instruction_line_size, 64);
        assert_eq!(cache.l2_size_kib, 512);
        assert_eq!(cache.l2_line_size, 64);
        assert_eq!(cache.l3_size_kib, 8192);
        assert_eq!(cache.l3_line_size, 64);
    }
}
