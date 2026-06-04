#![no_std]
#![forbid(unsafe_code)]

//! Hardware resource descriptors shared by Mirage driver services.
//!
//! This crate deliberately models hardware as supervisor-granted resources and
//! backend declarations. It does not perform raw device probing or expose a real
//! hardware backend by default; concrete drivers are expected to opt in through
//! the top-level `hw-*` feature flags and receive capabilities from the
//! supervisor before touching device resources.

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

/// Physical memory range granted to a driver service as an MMIO aperture.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MmioRegion {
    base: u64,
    length: u64,
}

impl MmioRegion {
    pub const fn new(base: u64, length: u64) -> Result<Self, HardwareError> {
        if length == 0 {
            Err(HardwareError::EmptyRange)
        } else {
            Ok(Self { base, length })
        }
    }

    pub const fn base(self) -> u64 {
        self.base
    }

    pub const fn length(self) -> u64 {
        self.length
    }

    pub const fn end_exclusive(self) -> Result<u64, HardwareError> {
        match self.base.checked_add(self.length) {
            Some(end) => Ok(end),
            None => Err(HardwareError::RangeOverflow),
        }
    }
}

/// DMA object made available to a service by the supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DmaRegion {
    id: HardwareResourceId,
    bytes: u64,
}

impl DmaRegion {
    pub const fn new(id: HardwareResourceId, bytes: u64) -> Result<Self, HardwareError> {
        if bytes == 0 {
            Err(HardwareError::EmptyRange)
        } else {
            Ok(Self { id, bytes })
        }
    }

    pub const fn id(self) -> HardwareResourceId {
        self.id
    }

    pub const fn bytes(self) -> u64 {
        self.bytes
    }
}

/// Interrupt line assigned to a driver service.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct IrqLine(u16);

impl IrqLine {
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Common capability-shaped hardware bundle for supervised driver services.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct HardwareResources {
    id: HardwareResourceId,
    mmio: MmioRegion,
    dma: DmaRegion,
    irq: IrqLine,
}

impl HardwareResources {
    pub const fn new(
        id: HardwareResourceId,
        mmio: MmioRegion,
        dma: DmaRegion,
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

    pub const fn dma(self) -> DmaRegion {
        self.dma
    }

    pub const fn irq(self) -> IrqLine {
        self.irq
    }
}

/// Errors returned by hardware descriptor validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum HardwareError {
    EmptyRange,
    RangeOverflow,
    BackendDisabled,
}
