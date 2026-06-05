//! AMD topology parsing from injectable CPUID leaves.

use crate::cpuid::{AmdCpuId, AmdCpuidReader};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdCoreId(pub u16);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdThreadId(pub u16);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AmdPackageId(pub u16);

/// Topology facts for the currently executing logical CPU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AmdTopology {
    pub apic_id: u32,
    pub x2apic_id: u32,
    pub package_id: AmdPackageId,
    pub core_id: AmdCoreId,
    pub thread_id: AmdThreadId,
    pub cores_per_package: u16,
    pub threads_per_core: u16,
    pub logical_processors_per_package: u16,
}

impl AmdTopology {
    /// Discover topology from hardware CPUID when enabled.
    pub fn discover() -> Self {
        let cpuid = AmdCpuId::read();
        Self::from_cpuid(cpuid)
    }

    /// Discover topology from an injectable CPUID reader.
    pub fn discover_from(reader: &impl AmdCpuidReader) -> Self {
        Self::from_cpuid(AmdCpuId::from_reader(reader))
    }

    pub const fn logical_cpus_per_package(self) -> u16 {
        self.logical_processors_per_package
    }

    pub const fn from_cpuid(cpuid: AmdCpuId) -> Self {
        let leaf1 = cpuid.leaf(0x0000_0001);
        let ext8 = cpuid.leaf(0x8000_0008);
        let ext1e = cpuid.leaf(0x8000_001e);

        let leaf1_apic_id = (leaf1.ebx >> 24) & 0xff;
        let leaf1_logical = ((leaf1.ebx >> 16) & 0xff) as u16;

        let apic_id = if ext1e.eax != 0 {
            ext1e.eax
        } else {
            leaf1_apic_id
        };
        let x2apic_id = apic_id;
        let core_id = if ext1e.ebx != 0 {
            (ext1e.ebx & 0xff) as u16
        } else {
            0
        };
        let threads_per_core = if ext1e.ebx != 0 {
            (((ext1e.ebx >> 8) & 0xff) + 1) as u16
        } else {
            1
        };
        let cores_per_package = ((ext8.ecx & 0xff) as u16) + 1;
        let logical_processors_per_package = if leaf1_logical != 0 {
            leaf1_logical
        } else {
            cores_per_package.saturating_mul(threads_per_core)
        };
        let thread_id = if threads_per_core > 1 {
            (apic_id as u16) % threads_per_core
        } else {
            0
        };
        let package_id = if ext1e.ecx != 0 {
            (ext1e.ecx & 0xff) as u16
        } else if logical_processors_per_package > 0 {
            (apic_id as u16) / logical_processors_per_package
        } else {
            0
        };

        Self {
            apic_id,
            x2apic_id,
            package_id: AmdPackageId(package_id),
            core_id: AmdCoreId(core_id),
            thread_id: AmdThreadId(thread_id),
            cores_per_package,
            threads_per_core,
            logical_processors_per_package,
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
                0x0000_0000 => CpuidLeaf::new(0x0000_0001, 0, 0, 0),
                0x0000_0001 => CpuidLeaf::new(0, (8 << 16) | (0x23 << 24), 0, 0),
                0x8000_0000 => CpuidLeaf::new(0x8000_001e, 0, 0, 0),
                0x8000_0008 => CpuidLeaf::new(0, 0, 3, 0),
                0x8000_001e => CpuidLeaf::new(0x23, 2 | (2 << 8), 1, 0),
                _ => CpuidLeaf::new(0, 0, 0, 0),
            }
        }
    }

    #[test]
    fn topology_parser_uses_amd_extended_topology_leaf() {
        let topology = AmdTopology::discover_from(&MockCpuid);

        assert_eq!(topology.apic_id, 0x23);
        assert_eq!(topology.x2apic_id, 0x23);
        assert_eq!(topology.package_id, AmdPackageId(1));
        assert_eq!(topology.core_id, AmdCoreId(2));
        assert_eq!(topology.thread_id, AmdThreadId(2));
        assert_eq!(topology.cores_per_package, 4);
        assert_eq!(topology.threads_per_core, 3);
        assert_eq!(topology.logical_processors_per_package, 8);
    }
}
