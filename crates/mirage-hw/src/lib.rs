#![no_std]

//! Hardware resource descriptors shared by Mirage driver services.
//!
//! This crate deliberately models hardware as supervisor-granted resources and
//! backend declarations. It does not perform raw device probing or expose a real
//! hardware backend by default; concrete drivers are expected to opt in through
//! the top-level `hw-*` feature flags and receive capabilities from the
//! supervisor before touching device resources.

use core::ptr;

use mirage_cap::{CapabilityObject, CapabilityRights, CapabilitySet};

/// Execution backend selected for a Mirage driver or hardware-facing service.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum HardwareBackend {
    /// Deterministic mock backend used by default builds and architecture tests.
    Mock,
    /// Real hardware backend gated by an explicit `hw-*` feature and supervisor policy.
    Hardware,
}

impl HardwareBackend {
    /// Returns true when this backend is the default mock implementation.
    pub const fn is_mock(self) -> bool {
        matches!(self, Self::Mock)
    }

    /// Returns true when this backend represents a feature-gated hardware path.
    pub const fn is_hardware(self) -> bool {
        matches!(self, Self::Hardware)
    }
}

/// Stable identifier for a supervisor-visible hardware resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct HardwareResourceId(u64);

impl HardwareResourceId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Physical address for a supervisor-granted hardware resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PhysAddr(u64);

impl PhysAddr {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn checked_add(self, offset: u64) -> Result<Self, HardwareError> {
        match self.0.checked_add(offset) {
            Some(address) => Ok(Self(address)),
            None => Err(HardwareError::RangeOverflow),
        }
    }
}

/// Virtual address where a supervisor-granted hardware resource is mapped.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct VirtAddr(usize);

impl VirtAddr {
    pub const fn new(raw: usize) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> usize {
        self.0
    }

    pub const fn checked_add(self, offset: usize) -> Result<Self, HardwareError> {
        match self.0.checked_add(offset) {
            Some(address) => Ok(Self(address)),
            None => Err(HardwareError::RangeOverflow),
        }
    }
}

/// Common MMIO access interface for real and mock-backed MMIO regions.
pub trait Mmio {
    fn read8(&self, offset: usize) -> Result<u8, HardwareError>;
    fn read16(&self, offset: usize) -> Result<u16, HardwareError>;
    fn read32(&self, offset: usize) -> Result<u32, HardwareError>;
    fn read64(&self, offset: usize) -> Result<u64, HardwareError>;

    fn write8(&mut self, offset: usize, value: u8) -> Result<(), HardwareError>;
    fn write16(&mut self, offset: usize, value: u16) -> Result<(), HardwareError>;
    fn write32(&mut self, offset: usize, value: u32) -> Result<(), HardwareError>;
    fn write64(&mut self, offset: usize, value: u64) -> Result<(), HardwareError>;
}

/// Physical memory range granted to a driver service as an MMIO aperture.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MmioRegion {
    phys_base: PhysAddr,
    virt_base: VirtAddr,
    length: usize,
}

impl MmioRegion {
    /// Constructs an MMIO aperture after validating supervisor-granted authority.
    pub fn new(
        phys_base: PhysAddr,
        virt_base: VirtAddr,
        length: usize,
        authority: &CapabilitySet,
    ) -> Result<Self, HardwareError> {
        if length == 0 {
            return Err(HardwareError::EmptyRange);
        }

        let length_u64 = u64::try_from(length).map_err(|_| HardwareError::RangeOverflow)?;
        phys_base.checked_add(length_u64)?;
        virt_base.checked_add(length)?;
        authority.check(
            CapabilityObject::MmioRegion {
                base: phys_base.get(),
                length: length_u64,
            },
            CapabilityRights::read_write_io(),
        )?;

        Ok(Self {
            phys_base,
            virt_base,
            length,
        })
    }

    pub const fn phys_base(self) -> PhysAddr {
        self.phys_base
    }

    pub const fn virt_base(self) -> VirtAddr {
        self.virt_base
    }

    /// Backward-compatible accessor for the physical base address.
    pub const fn base(self) -> u64 {
        self.phys_base.get()
    }

    pub const fn length(self) -> usize {
        self.length
    }

    pub const fn end_exclusive(self) -> Result<u64, HardwareError> {
        match self.phys_base.get().checked_add(self.length as u64) {
            Some(end) => Ok(end),
            None => Err(HardwareError::RangeOverflow),
        }
    }

    fn access_ptr<T>(&self, offset: usize) -> Result<*mut T, HardwareError> {
        let width = core::mem::size_of::<T>();
        let end = offset
            .checked_add(width)
            .ok_or(HardwareError::RangeOverflow)?;
        if end > self.length {
            return Err(HardwareError::OutOfRange);
        }
        if offset % core::mem::align_of::<T>() != 0 {
            return Err(HardwareError::MisalignedAccess);
        }

        let address = self
            .virt_base
            .get()
            .checked_add(offset)
            .ok_or(HardwareError::RangeOverflow)?;
        Ok(address as *mut T)
    }

    pub fn read8(&self, offset: usize) -> Result<u8, HardwareError> {
        <Self as Mmio>::read8(self, offset)
    }

    pub fn read16(&self, offset: usize) -> Result<u16, HardwareError> {
        <Self as Mmio>::read16(self, offset)
    }

    pub fn read32(&self, offset: usize) -> Result<u32, HardwareError> {
        <Self as Mmio>::read32(self, offset)
    }

    pub fn read64(&self, offset: usize) -> Result<u64, HardwareError> {
        <Self as Mmio>::read64(self, offset)
    }

    pub fn write8(&mut self, offset: usize, value: u8) -> Result<(), HardwareError> {
        <Self as Mmio>::write8(self, offset, value)
    }

    pub fn write16(&mut self, offset: usize, value: u16) -> Result<(), HardwareError> {
        <Self as Mmio>::write16(self, offset, value)
    }

    pub fn write32(&mut self, offset: usize, value: u32) -> Result<(), HardwareError> {
        <Self as Mmio>::write32(self, offset, value)
    }

    pub fn write64(&mut self, offset: usize, value: u64) -> Result<(), HardwareError> {
        <Self as Mmio>::write64(self, offset, value)
    }
}

impl Mmio for MmioRegion {
    fn read8(&self, offset: usize) -> Result<u8, HardwareError> {
        let pointer = self.access_ptr::<u8>(offset)?;
        // SAFETY: `access_ptr` bounds-checks the offset against the supervisor-granted
        // mapping length and u8 has alignment 1; volatile access is required for MMIO.
        Ok(unsafe { ptr::read_volatile(pointer.cast_const()) })
    }

    fn read16(&self, offset: usize) -> Result<u16, HardwareError> {
        let pointer = self.access_ptr::<u16>(offset)?;
        // SAFETY: `access_ptr` verifies range and alignment for u16 within the MMIO mapping;
        // volatile access prevents the compiler from eliding device register reads.
        Ok(unsafe { ptr::read_volatile(pointer.cast_const()) })
    }

    fn read32(&self, offset: usize) -> Result<u32, HardwareError> {
        let pointer = self.access_ptr::<u32>(offset)?;
        // SAFETY: `access_ptr` verifies range and alignment for u32 within the MMIO mapping;
        // volatile access is the correct primitive for hardware register reads.
        Ok(unsafe { ptr::read_volatile(pointer.cast_const()) })
    }

    fn read64(&self, offset: usize) -> Result<u64, HardwareError> {
        let pointer = self.access_ptr::<u64>(offset)?;
        // SAFETY: `access_ptr` verifies range and alignment for u64 within the MMIO mapping;
        // volatile access is the correct primitive for hardware register reads.
        Ok(unsafe { ptr::read_volatile(pointer.cast_const()) })
    }

    fn write8(&mut self, offset: usize, value: u8) -> Result<(), HardwareError> {
        let pointer = self.access_ptr::<u8>(offset)?;
        // SAFETY: `access_ptr` bounds-checks the offset against the MMIO mapping and u8 has
        // alignment 1; volatile access is required for device register writes.
        unsafe { ptr::write_volatile(pointer, value) };
        Ok(())
    }

    fn write16(&mut self, offset: usize, value: u16) -> Result<(), HardwareError> {
        let pointer = self.access_ptr::<u16>(offset)?;
        // SAFETY: `access_ptr` verifies range and alignment for u16 within the MMIO mapping;
        // volatile access prevents the compiler from eliding device register writes.
        unsafe { ptr::write_volatile(pointer, value) };
        Ok(())
    }

    fn write32(&mut self, offset: usize, value: u32) -> Result<(), HardwareError> {
        let pointer = self.access_ptr::<u32>(offset)?;
        // SAFETY: `access_ptr` verifies range and alignment for u32 within the MMIO mapping;
        // volatile access is required for hardware register writes.
        unsafe { ptr::write_volatile(pointer, value) };
        Ok(())
    }

    fn write64(&mut self, offset: usize, value: u64) -> Result<(), HardwareError> {
        let pointer = self.access_ptr::<u64>(offset)?;
        // SAFETY: `access_ptr` verifies range and alignment for u64 within the MMIO mapping;
        // volatile access is required for hardware register writes.
        unsafe { ptr::write_volatile(pointer, value) };
        Ok(())
    }
}

/// Direction for a DMA buffer from the device's perspective.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DmaDirection {
    ToDevice,
    FromDevice,
    Bidirectional,
}

/// DMA object made available to a service by the supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DmaBuffer {
    id: HardwareResourceId,
    phys_base: PhysAddr,
    virt_base: VirtAddr,
    bytes: usize,
    direction: DmaDirection,
}

impl DmaBuffer {
    pub fn new(
        id: HardwareResourceId,
        phys_base: PhysAddr,
        virt_base: VirtAddr,
        bytes: usize,
        direction: DmaDirection,
        authority: &CapabilitySet,
    ) -> Result<Self, HardwareError> {
        if bytes == 0 {
            return Err(HardwareError::EmptyRange);
        }

        let bytes_u64 = u64::try_from(bytes).map_err(|_| HardwareError::RangeOverflow)?;
        phys_base.checked_add(bytes_u64)?;
        virt_base.checked_add(bytes)?;
        authority.check(
            CapabilityObject::DmaRegion(id.get()),
            CapabilityRights::read_write_io(),
        )?;

        Ok(Self {
            id,
            phys_base,
            virt_base,
            bytes,
            direction,
        })
    }

    pub const fn id(self) -> HardwareResourceId {
        self.id
    }

    pub const fn phys_base(self) -> PhysAddr {
        self.phys_base
    }

    pub const fn virt_base(self) -> VirtAddr {
        self.virt_base
    }

    pub const fn bytes(self) -> usize {
        self.bytes
    }

    pub const fn direction(self) -> DmaDirection {
        self.direction
    }
}

/// Backward-compatible DMA region name used by existing Mirage skeleton code.
pub type DmaRegion = DmaBuffer;

/// Interrupt line assigned to a driver service.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct IrqLine(u16);

impl IrqLine {
    pub fn new(raw: u16, authority: &CapabilitySet) -> Result<Self, HardwareError> {
        authority.check(CapabilityObject::IrqLine(raw), CapabilityRights::io())?;
        Ok(Self(raw))
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

/// PCI base address register descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PciBar {
    index: u8,
    base: PhysAddr,
    length: usize,
    kind: PciBarKind,
}

impl PciBar {
    pub const MAX_INDEX: u8 = 5;

    pub const fn new(
        index: u8,
        base: PhysAddr,
        length: usize,
        kind: PciBarKind,
    ) -> Result<Self, HardwareError> {
        if index > Self::MAX_INDEX {
            Err(HardwareError::InvalidPciBar)
        } else if length == 0 {
            Err(HardwareError::EmptyRange)
        } else {
            Ok(Self {
                index,
                base,
                length,
                kind,
            })
        }
    }

    pub const fn index(self) -> u8 {
        self.index
    }

    pub const fn base(self) -> PhysAddr {
        self.base
    }

    pub const fn length(self) -> usize {
        self.length
    }

    pub const fn kind(self) -> PciBarKind {
        self.kind
    }
}

/// PCI BAR address space type.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum PciBarKind {
    Memory32,
    Memory64,
    IoPort,
}

/// Common capability-shaped hardware bundle for supervised driver services.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct HardwareResources {
    id: HardwareResourceId,
    mmio: MmioRegion,
    dma: DmaBuffer,
    irq: IrqLine,
}

impl HardwareResources {
    pub const fn new(
        id: HardwareResourceId,
        mmio: MmioRegion,
        dma: DmaBuffer,
        irq: IrqLine,
    ) -> Self {
        Self { id, mmio, dma, irq }
    }

    pub const fn id(self) -> HardwareResourceId {
        self.id
    }

    pub const fn mmio(self) -> MmioRegion {
        self.mmio
    }

    pub const fn dma(self) -> DmaBuffer {
        self.dma
    }

    pub const fn irq(self) -> IrqLine {
        self.irq
    }
}

/// Errors returned by hardware descriptor validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HardwareError {
    EmptyRange,
    RangeOverflow,
    OutOfRange,
    MisalignedAccess,
    InvalidPciBar,
    BackendDisabled,
    MissingCapability,
    InsufficientCapabilityRights,
    RevokedCapability,
}

impl From<mirage_cap::CapabilityError> for HardwareError {
    fn from(error: mirage_cap::CapabilityError) -> Self {
        match error {
            mirage_cap::CapabilityError::Missing => Self::MissingCapability,
            mirage_cap::CapabilityError::InsufficientRights => Self::InsufficientCapabilityRights,
            mirage_cap::CapabilityError::Revoked => Self::RevokedCapability,
        }
    }
}

#[cfg(any(test, feature = "mock"))]
pub mod mock {
    use super::{HardwareError, Mmio};

    /// Memory-backed MMIO implementation for deterministic driver tests.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct MockMmioRegion<const N: usize> {
        memory: [u8; N],
    }

    impl<const N: usize> MockMmioRegion<N> {
        pub const fn new() -> Self {
            Self { memory: [0; N] }
        }

        pub const fn from_bytes(memory: [u8; N]) -> Self {
            Self { memory }
        }

        pub const fn as_bytes(&self) -> &[u8; N] {
            &self.memory
        }

        fn access(&self, offset: usize, width: usize) -> Result<(), HardwareError> {
            let end = offset
                .checked_add(width)
                .ok_or(HardwareError::RangeOverflow)?;
            if end > N {
                return Err(HardwareError::OutOfRange);
            }
            if offset % width != 0 {
                return Err(HardwareError::MisalignedAccess);
            }
            Ok(())
        }
    }

    impl<const N: usize> Default for MockMmioRegion<N> {
        fn default() -> Self {
            Self::new()
        }
    }

    impl<const N: usize> Mmio for MockMmioRegion<N> {
        fn read8(&self, offset: usize) -> Result<u8, HardwareError> {
            self.access(offset, 1)?;
            Ok(self.memory[offset])
        }

        fn read16(&self, offset: usize) -> Result<u16, HardwareError> {
            self.access(offset, 2)?;
            Ok(u16::from_le_bytes([
                self.memory[offset],
                self.memory[offset + 1],
            ]))
        }

        fn read32(&self, offset: usize) -> Result<u32, HardwareError> {
            self.access(offset, 4)?;
            Ok(u32::from_le_bytes([
                self.memory[offset],
                self.memory[offset + 1],
                self.memory[offset + 2],
                self.memory[offset + 3],
            ]))
        }

        fn read64(&self, offset: usize) -> Result<u64, HardwareError> {
            self.access(offset, 8)?;
            Ok(u64::from_le_bytes([
                self.memory[offset],
                self.memory[offset + 1],
                self.memory[offset + 2],
                self.memory[offset + 3],
                self.memory[offset + 4],
                self.memory[offset + 5],
                self.memory[offset + 6],
                self.memory[offset + 7],
            ]))
        }

        fn write8(&mut self, offset: usize, value: u8) -> Result<(), HardwareError> {
            self.access(offset, 1)?;
            self.memory[offset] = value;
            Ok(())
        }

        fn write16(&mut self, offset: usize, value: u16) -> Result<(), HardwareError> {
            self.access(offset, 2)?;
            self.memory[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
            Ok(())
        }

        fn write32(&mut self, offset: usize, value: u32) -> Result<(), HardwareError> {
            self.access(offset, 4)?;
            self.memory[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            Ok(())
        }

        fn write64(&mut self, offset: usize, value: u64) -> Result<(), HardwareError> {
            self.access(offset, 8)?;
            self.memory[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::vec;

    use super::*;
    use mirage_cap::{Capability, CapabilityRight};

    fn capabilities() -> CapabilitySet {
        CapabilitySet::from_capabilities(vec![
            Capability::new(
                CapabilityObject::MmioRegion {
                    base: 0x1000,
                    length: 0x100,
                },
                CapabilityRights::read_write_io(),
            ),
            Capability::new(
                CapabilityObject::DmaRegion(7),
                CapabilityRights::read_write_io(),
            ),
            Capability::new(CapabilityObject::IrqLine(11), CapabilityRights::io()),
        ])
    }

    #[test]
    fn constructors_require_capabilities() {
        let authority = capabilities();
        assert!(MmioRegion::new(
            PhysAddr::new(0x1000),
            VirtAddr::new(0x2000),
            0x100,
            &authority,
        )
        .is_ok());
        assert_eq!(
            MmioRegion::new(
                PhysAddr::new(0x2000),
                VirtAddr::new(0x2000),
                0x100,
                &authority,
            ),
            Err(HardwareError::MissingCapability)
        );
        assert!(DmaBuffer::new(
            HardwareResourceId::new(7),
            PhysAddr::new(0x3000),
            VirtAddr::new(0x4000),
            0x80,
            DmaDirection::Bidirectional,
            &authority,
        )
        .is_ok());
        assert!(IrqLine::new(11, &authority).is_ok());
    }

    #[test]
    fn constructor_rejects_insufficient_rights() {
        let authority = CapabilitySet::from_capabilities(vec![Capability::new(
            CapabilityObject::MmioRegion {
                base: 0x1000,
                length: 0x100,
            },
            CapabilityRights::empty().with(CapabilityRight::Read),
        )]);

        assert_eq!(
            MmioRegion::new(
                PhysAddr::new(0x1000),
                VirtAddr::new(0x2000),
                0x100,
                &authority,
            ),
            Err(HardwareError::InsufficientCapabilityRights)
        );
    }

    #[test]
    fn mock_mmio_reads_and_writes_all_widths() {
        let mut mmio = mock::MockMmioRegion::<32>::new();

        mmio.write8(0, 0xab).unwrap();
        mmio.write16(2, 0xcdef).unwrap();
        mmio.write32(4, 0x1234_5678).unwrap();
        mmio.write64(8, 0x1122_3344_5566_7788).unwrap();

        assert_eq!(mmio.read8(0).unwrap(), 0xab);
        assert_eq!(mmio.read16(2).unwrap(), 0xcdef);
        assert_eq!(mmio.read32(4).unwrap(), 0x1234_5678);
        assert_eq!(mmio.read64(8).unwrap(), 0x1122_3344_5566_7788);
    }

    #[test]
    fn mock_mmio_rejects_invalid_access() {
        let mut mmio = mock::MockMmioRegion::<8>::new();

        assert_eq!(mmio.read32(6), Err(HardwareError::OutOfRange));
        assert_eq!(mmio.read16(1), Err(HardwareError::MisalignedAccess));
        assert_eq!(mmio.write64(4, 0), Err(HardwareError::OutOfRange));
    }
}
