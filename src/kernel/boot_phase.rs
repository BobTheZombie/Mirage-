//! Canonical no-heap boot subsystem registration and status tracking.
//!
//! Mirage's early boot path uses this module as the single source of truth for
//! subsystem visibility.  Every subsystem registers a static descriptor before
//! it starts, then reports state transitions through the functions below.  The
//! table is fixed-size, `no_std` friendly, and queryable by the framebuffer boot
//! screen and future debug shell without requiring heap allocation.

use crate::kernel::sync::SpinLock;

/// Maximum number of boot subsystem records tracked without allocation.
pub const BOOT_PHASE_CAPACITY: usize = 64;
/// Current number of canonical Mirage boot phases.
pub const BOOT_PHASE_COUNT: usize = 63;

/// Coarse subsystem ownership used by boot rendering and future debug queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubsystemCategory {
    Seed,
    Boot,
    Architecture,
    Memory,
    Device,
    Input,
    Storage,
    Supervisor,
    Userspace,
    Scheduler,
    Debug,
}

/// Ordered Mirage boot phases tracked by the boot phase manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BootPhase {
    SeedRs,
    BootInfo,
    KernelMain,
    Architecture,
    Serial,
    Gdt,
    MemoryMap,
    PhysicalAllocator,
    KernelMapper,
    Paging,
    Heap,
    Memory,
    Framebuffer,
    Idt,
    Pic,
    Interrupts,
    Amd64Cpu,
    RyzenCpu,
    RyzenTopology,
    AmdSoc,
    AmdIommu,
    AcpiTables,
    Thermal,
    Battery,
    AmdGpuRenoir,
    AmdXhci,
    BlockLayer,
    M2Storage,
    Nvme,
    NvmeNamespace,
    Ahci,
    SataDisk,
    Atapi,
    OpticalDisk,
    PartitionLayer,
    Mbr,
    Gpt,
    Qfs,
    Ext4,
    I8042,
    Ps2Keyboard,
    Xhci,
    UsbCore,
    UsbHid,
    UsbKeyboard,
    AcpiEc,
    EcHotkeys,
    Input,
    KernelConstructed,
    BootInfoApplied,
    SupervisorCreated,
    RootFs,
    Supervisor,
    UserspaceLoader,
    SpiderRs,
    Pid1,
    SystemDispatcher,
    M1Terminal,
    Userspace,
    BootRuntime,
    Mtss,
    BootScreen,
    IdleLoop,
}

impl BootPhase {
    /// Stable descriptor name for fallback registration.
    pub const fn name(self) -> &'static str {
        fallback_descriptor(self).name
    }

    /// Friendly current-phase message for the persistent boot screen.
    pub const fn friendly_name(self) -> &'static str {
        match self {
            Self::Amd64Cpu => "AMD64 CPU",
            Self::RyzenCpu => "Ryzen CPU",
            Self::RyzenTopology => "Ryzen Topology",
            Self::AmdSoc => "AMD SoC",
            Self::AmdIommu => "AMD IOMMU",
            Self::AcpiTables => "ACPI Tables",
            Self::AmdGpuRenoir => "AMDGPU Renoir",
            Self::AmdXhci => "AMD xHCI",
            Self::BlockLayer => "Block Layer",
            Self::M2Storage => "M.2-capable storage path",
            Self::Nvme => "NVMe",
            Self::NvmeNamespace => "NVMe Namespace",
            Self::Ahci => "AHCI",
            Self::SataDisk => "SATA Disk",
            Self::Atapi => "ATAPI",
            Self::OpticalDisk => "Optical Disk",
            Self::PartitionLayer => "Partition Layer",
            Self::Mbr => "MBR",
            Self::Gpt => "GPT",
            Self::Qfs => "QFS",
            Self::Ext4 => "ext4",
            Self::UsbCore => "USB Core",
            Self::UsbHid => "USB HID",
            Self::UsbKeyboard => "USB Keyboard",
            Self::KernelMapper => "Kernel Mapper",
            Self::RootFs => "Root FS",
            Self::UserspaceLoader => "Userspace Loader",
            Self::SpiderRs => "Spider-rs",
            Self::Pid1 => "PID1",
            Self::SystemDispatcher => "System Dispatcher",
            Self::M1Terminal => "M1 Terminal",
            Self::BootRuntime => "Boot Runtime",
            Self::Mtss => "MTSS",
            _ => self.name(),
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// State of a tracked boot phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhaseState {
    Unregistered,
    Registered,
    Pending,
    Started,
    Detected,
    Found,
    Ok,
    Online,
    Enabled,
    Stub,
    Skipped,
    Failed,
    Running,
}

impl PhaseState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unregistered => "Unregistered",
            Self::Registered => "Registered",
            Self::Pending => "Pending",
            Self::Started => "Started",
            Self::Detected => "Detected",
            Self::Found => "Found",
            Self::Ok => "Ok",
            Self::Online => "Online",
            Self::Enabled => "Enabled",
            Self::Stub => "Stub",
            Self::Skipped => "Skipped",
            Self::Failed => "Failed",
            Self::Running => "Running",
        }
    }

    const fn weighted_progress(self, required: bool, weight: u8) -> u16 {
        let weight = weight as u16;
        match self {
            Self::Ok | Self::Online | Self::Enabled | Self::Running => weight,
            Self::Skipped | Self::Detected | Self::Found | Self::Stub => {
                if required {
                    weight / 2
                } else {
                    weight
                }
            }
            Self::Started => (weight + 1) / 2,
            Self::Unregistered | Self::Registered | Self::Pending | Self::Failed => 0,
        }
    }
}

/// Static subsystem metadata registered before initialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubsystemDescriptor {
    pub phase: BootPhase,
    pub name: &'static str,
    pub category: SubsystemCategory,
    pub required: bool,
    pub weight: u8,
}

/// Fixed record for a boot phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootPhaseRecord {
    pub descriptor: SubsystemDescriptor,
    pub state: PhaseState,
    pub message: &'static str,
}

/// No-heap boot phase manager with a fixed static phase table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootPhaseManager {
    records: [BootPhaseRecord; BOOT_PHASE_CAPACITY],
    pub current_phase: BootPhase,
}

impl BootPhaseManager {
    pub const fn new() -> Self {
        Self {
            records: [unregistered_record(); BOOT_PHASE_CAPACITY],
            current_phase: BootPhase::SeedRs,
        }
    }

    pub fn register(&mut self, descriptor: SubsystemDescriptor) -> bool {
        let index = descriptor.phase.index();
        let duplicate = self.records[index].state != PhaseState::Unregistered;
        if !duplicate {
            self.records[index] = BootPhaseRecord {
                descriptor,
                state: PhaseState::Registered,
                message: "registered",
            };
        }
        duplicate
    }

    pub fn mark_registered_pending(&mut self) {
        let mut index = 0usize;
        while index < BOOT_PHASE_CAPACITY {
            if self.records[index].state == PhaseState::Registered {
                self.records[index].state = PhaseState::Pending;
                self.records[index].message = "pending";
            }
            index += 1;
        }
    }

    pub fn transition(
        &mut self,
        phase: BootPhase,
        state: PhaseState,
        message: &'static str,
    ) -> bool {
        let index = phase.index();
        if self.records[index].state == PhaseState::Unregistered {
            return false;
        }
        self.current_phase = phase;
        self.records[index].state = state;
        self.records[index].message = message;
        true
    }

    pub const fn state(&self, phase: BootPhase) -> PhaseState {
        self.records[phase.index()].state
    }

    pub const fn record(&self, phase: BootPhase) -> Option<&BootPhaseRecord> {
        let record = &self.records[phase.index()];
        match record.state {
            PhaseState::Unregistered => None,
            _ => Some(record),
        }
    }

    pub fn for_each_record(&self, mut visit: impl FnMut(&BootPhaseRecord)) {
        let mut index = 0usize;
        while index < BOOT_PHASE_CAPACITY {
            let record = &self.records[index];
            if record.state != PhaseState::Unregistered {
                visit(record);
            }
            index += 1;
        }
    }

    pub fn progress_percent(&self) -> u8 {
        let mut index = 0usize;
        let mut completed = 0u16;
        let mut total = 0u16;
        while index < BOOT_PHASE_CAPACITY {
            let record = self.records[index];
            if record.state != PhaseState::Unregistered {
                total += record.descriptor.weight as u16;
                completed += record
                    .state
                    .weighted_progress(record.descriptor.required, record.descriptor.weight);
            }
            index += 1;
        }
        if total == 0 {
            0
        } else {
            ((completed as u32 * 100) / total as u32) as u8
        }
    }

    pub fn has_failed(&self) -> bool {
        let mut index = 0usize;
        while index < BOOT_PHASE_CAPACITY {
            if self.records[index].state == PhaseState::Failed {
                return true;
            }
            index += 1;
        }
        false
    }

    pub fn validate_no_unresolved(&mut self) {
        let mut index = 0usize;
        while index < BOOT_PHASE_CAPACITY {
            let record = &mut self.records[index];
            match record.state {
                PhaseState::Registered | PhaseState::Pending => {
                    if record.descriptor.required {
                        record.state = PhaseState::Failed;
                        record.message = "required phase not reached";
                    } else {
                        record.state = PhaseState::Skipped;
                        record.message = "not present/not probed";
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }
}

const fn unregistered_record() -> BootPhaseRecord {
    BootPhaseRecord {
        descriptor: SubsystemDescriptor {
            phase: BootPhase::SeedRs,
            name: "Unregistered",
            category: SubsystemCategory::Debug,
            required: false,
            weight: 0,
        },
        state: PhaseState::Unregistered,
        message: "unregistered",
    }
}

pub const DEFAULT_SUBSYSTEM_DESCRIPTORS: [SubsystemDescriptor; BOOT_PHASE_COUNT] = [
    descriptor(
        BootPhase::SeedRs,
        "Seed-rs",
        SubsystemCategory::Seed,
        true,
        5,
    ),
    descriptor(
        BootPhase::BootInfo,
        "BootInfo",
        SubsystemCategory::Boot,
        true,
        5,
    ),
    descriptor(
        BootPhase::KernelMain,
        "KernelMain",
        SubsystemCategory::Boot,
        true,
        3,
    ),
    descriptor(
        BootPhase::Architecture,
        "Architecture",
        SubsystemCategory::Architecture,
        true,
        5,
    ),
    descriptor(
        BootPhase::Serial,
        "Serial",
        SubsystemCategory::Architecture,
        true,
        3,
    ),
    descriptor(
        BootPhase::Gdt,
        "GDT",
        SubsystemCategory::Architecture,
        true,
        3,
    ),
    descriptor(
        BootPhase::MemoryMap,
        "MemoryMap",
        SubsystemCategory::Memory,
        true,
        4,
    ),
    descriptor(
        BootPhase::PhysicalAllocator,
        "PhysicalAllocator",
        SubsystemCategory::Memory,
        true,
        5,
    ),
    descriptor(
        BootPhase::KernelMapper,
        "Kernel Mapper",
        SubsystemCategory::Memory,
        true,
        5,
    ),
    descriptor(
        BootPhase::Paging,
        "Paging",
        SubsystemCategory::Memory,
        true,
        5,
    ),
    descriptor(BootPhase::Heap, "Heap", SubsystemCategory::Memory, true, 5),
    descriptor(
        BootPhase::Memory,
        "Memory",
        SubsystemCategory::Memory,
        true,
        5,
    ),
    descriptor(
        BootPhase::Framebuffer,
        "Framebuffer",
        SubsystemCategory::Device,
        false,
        3,
    ),
    descriptor(
        BootPhase::Idt,
        "IDT",
        SubsystemCategory::Architecture,
        true,
        3,
    ),
    descriptor(
        BootPhase::Pic,
        "PIC",
        SubsystemCategory::Architecture,
        true,
        3,
    ),
    descriptor(
        BootPhase::Interrupts,
        "Interrupts",
        SubsystemCategory::Architecture,
        true,
        4,
    ),
    descriptor(
        BootPhase::Amd64Cpu,
        "AMD64 CPU",
        SubsystemCategory::Architecture,
        false,
        3,
    ),
    descriptor(
        BootPhase::RyzenCpu,
        "Ryzen CPU",
        SubsystemCategory::Architecture,
        false,
        3,
    ),
    descriptor(
        BootPhase::RyzenTopology,
        "Ryzen Topology",
        SubsystemCategory::Scheduler,
        false,
        2,
    ),
    descriptor(
        BootPhase::AmdSoc,
        "AMD SoC",
        SubsystemCategory::Device,
        false,
        3,
    ),
    descriptor(
        BootPhase::AmdIommu,
        "AMD IOMMU",
        SubsystemCategory::Device,
        false,
        3,
    ),
    descriptor(
        BootPhase::AcpiTables,
        "ACPI Tables",
        SubsystemCategory::Architecture,
        false,
        3,
    ),
    descriptor(
        BootPhase::Thermal,
        "Thermal",
        SubsystemCategory::Device,
        false,
        2,
    ),
    descriptor(
        BootPhase::Battery,
        "Battery",
        SubsystemCategory::Device,
        false,
        2,
    ),
    descriptor(
        BootPhase::AmdGpuRenoir,
        "AMDGPU Renoir",
        SubsystemCategory::Device,
        false,
        3,
    ),
    descriptor(
        BootPhase::AmdXhci,
        "AMD xHCI",
        SubsystemCategory::Device,
        false,
        3,
    ),
    descriptor(
        BootPhase::BlockLayer,
        "Block Layer",
        SubsystemCategory::Storage,
        false,
        3,
    ),
    descriptor(
        BootPhase::M2Storage,
        "M.2-capable storage path",
        SubsystemCategory::Storage,
        false,
        2,
    ),
    descriptor(
        BootPhase::Nvme,
        "NVMe",
        SubsystemCategory::Storage,
        false,
        3,
    ),
    descriptor(
        BootPhase::NvmeNamespace,
        "NVMe Namespace",
        SubsystemCategory::Storage,
        false,
        3,
    ),
    descriptor(
        BootPhase::Ahci,
        "AHCI",
        SubsystemCategory::Storage,
        false,
        3,
    ),
    descriptor(
        BootPhase::SataDisk,
        "SATA Disk",
        SubsystemCategory::Storage,
        false,
        3,
    ),
    descriptor(
        BootPhase::Atapi,
        "ATAPI",
        SubsystemCategory::Storage,
        false,
        2,
    ),
    descriptor(
        BootPhase::OpticalDisk,
        "Optical Disk",
        SubsystemCategory::Storage,
        false,
        2,
    ),
    descriptor(
        BootPhase::PartitionLayer,
        "Partition Layer",
        SubsystemCategory::Storage,
        false,
        3,
    ),
    descriptor(BootPhase::Mbr, "MBR", SubsystemCategory::Storage, false, 2),
    descriptor(BootPhase::Gpt, "GPT", SubsystemCategory::Storage, false, 2),
    descriptor(BootPhase::Qfs, "QFS", SubsystemCategory::Storage, false, 3),
    descriptor(
        BootPhase::Ext4,
        "ext4",
        SubsystemCategory::Storage,
        false,
        3,
    ),
    descriptor(
        BootPhase::I8042,
        "I8042",
        SubsystemCategory::Input,
        false,
        2,
    ),
    descriptor(
        BootPhase::Ps2Keyboard,
        "PS/2 Keyboard",
        SubsystemCategory::Input,
        false,
        3,
    ),
    descriptor(
        BootPhase::Xhci,
        "AMD xHCI",
        SubsystemCategory::Device,
        false,
        4,
    ),
    descriptor(
        BootPhase::UsbCore,
        "USB Core",
        SubsystemCategory::Device,
        false,
        3,
    ),
    descriptor(
        BootPhase::UsbHid,
        "USB HID",
        SubsystemCategory::Input,
        false,
        3,
    ),
    descriptor(
        BootPhase::UsbKeyboard,
        "USB Keyboard",
        SubsystemCategory::Input,
        false,
        3,
    ),
    descriptor(
        BootPhase::AcpiEc,
        "ACPI EC",
        SubsystemCategory::Device,
        false,
        2,
    ),
    descriptor(
        BootPhase::EcHotkeys,
        "EC Hotkeys",
        SubsystemCategory::Input,
        false,
        2,
    ),
    descriptor(
        BootPhase::Input,
        "Input",
        SubsystemCategory::Input,
        false,
        3,
    ),
    descriptor(
        BootPhase::KernelConstructed,
        "KernelConstructed",
        SubsystemCategory::Boot,
        true,
        3,
    ),
    descriptor(
        BootPhase::BootInfoApplied,
        "BootInfoApplied",
        SubsystemCategory::Boot,
        true,
        3,
    ),
    descriptor(
        BootPhase::SupervisorCreated,
        "SupervisorCreated",
        SubsystemCategory::Supervisor,
        true,
        3,
    ),
    descriptor(
        BootPhase::RootFs,
        "Root FS",
        SubsystemCategory::Storage,
        true,
        5,
    ),
    descriptor(
        BootPhase::Supervisor,
        "Supervisor",
        SubsystemCategory::Supervisor,
        true,
        5,
    ),
    descriptor(
        BootPhase::UserspaceLoader,
        "Userspace Loader",
        SubsystemCategory::Userspace,
        false,
        3,
    ),
    descriptor(
        BootPhase::SpiderRs,
        "Spider-rs",
        SubsystemCategory::Userspace,
        false,
        2,
    ),
    descriptor(
        BootPhase::Pid1,
        "PID1",
        SubsystemCategory::Userspace,
        false,
        3,
    ),
    descriptor(
        BootPhase::SystemDispatcher,
        "System Dispatcher",
        SubsystemCategory::Userspace,
        false,
        3,
    ),
    descriptor(
        BootPhase::M1Terminal,
        "M1 Terminal",
        SubsystemCategory::Userspace,
        false,
        2,
    ),
    descriptor(
        BootPhase::Userspace,
        "Userspace",
        SubsystemCategory::Userspace,
        false,
        3,
    ),
    descriptor(
        BootPhase::BootRuntime,
        "Boot Runtime",
        SubsystemCategory::Userspace,
        true,
        4,
    ),
    descriptor(
        BootPhase::Mtss,
        "MTSS",
        SubsystemCategory::Scheduler,
        true,
        5,
    ),
    descriptor(
        BootPhase::BootScreen,
        "BootScreen",
        SubsystemCategory::Debug,
        false,
        1,
    ),
    descriptor(
        BootPhase::IdleLoop,
        "IdleLoop",
        SubsystemCategory::Scheduler,
        true,
        3,
    ),
];

const fn descriptor(
    phase: BootPhase,
    name: &'static str,
    category: SubsystemCategory,
    required: bool,
    weight: u8,
) -> SubsystemDescriptor {
    SubsystemDescriptor {
        phase,
        name,
        category,
        required,
        weight,
    }
}

const fn fallback_descriptor(phase: BootPhase) -> SubsystemDescriptor {
    DEFAULT_SUBSYSTEM_DESCRIPTORS[phase as usize]
}

static BOOT_PHASE_MANAGER: SpinLock<BootPhaseManager> = SpinLock::new(BootPhaseManager::new());

/// Register only subsystems compiled into this kernel build and leave them pending.
pub fn boot_register_compiled_subsystems() {
    register_phase(BootPhase::SeedRs);
    register_phase(BootPhase::BootInfo);
    register_phase(BootPhase::KernelMain);
    register_phase(BootPhase::Architecture);
    register_phase(BootPhase::Serial);
    register_phase(BootPhase::Gdt);
    register_phase(BootPhase::MemoryMap);
    register_phase(BootPhase::PhysicalAllocator);
    register_phase(BootPhase::KernelMapper);
    register_phase(BootPhase::Paging);
    register_phase(BootPhase::Heap);
    register_phase(BootPhase::Memory);
    #[cfg(feature = "hw-framebuffer")]
    register_phase(BootPhase::Framebuffer);
    register_phase(BootPhase::Idt);
    register_phase(BootPhase::Pic);
    register_phase(BootPhase::Interrupts);
    #[cfg(feature = "hw-amd64")]
    register_phase(BootPhase::Amd64Cpu);
    #[cfg(feature = "hw-ryzen")]
    {
        register_phase(BootPhase::RyzenCpu);
        register_phase(BootPhase::RyzenTopology);
    }
    #[cfg(feature = "hw-amd-chipset")]
    register_phase(BootPhase::AmdSoc);
    #[cfg(feature = "hw-amd-iommu")]
    register_phase(BootPhase::AmdIommu);
    #[cfg(feature = "hw-acpi")]
    register_phase(BootPhase::AcpiTables);
    #[cfg(feature = "hw-amd-telemetry")]
    {
        register_phase(BootPhase::Thermal);
        register_phase(BootPhase::Battery);
    }
    #[cfg(feature = "hw-amdgpu")]
    register_phase(BootPhase::AmdGpuRenoir);
    #[cfg(feature = "hw-xhci")]
    register_phase(BootPhase::AmdXhci);
    register_phase(BootPhase::BlockLayer);
    register_phase(BootPhase::M2Storage);
    #[cfg(feature = "hw-nvme")]
    {
        register_phase(BootPhase::Nvme);
        register_phase(BootPhase::NvmeNamespace);
    }
    #[cfg(feature = "hw-ahci")]
    {
        register_phase(BootPhase::Ahci);
        register_phase(BootPhase::SataDisk);
        register_phase(BootPhase::Atapi);
        register_phase(BootPhase::OpticalDisk);
    }
    register_phase(BootPhase::PartitionLayer);
    register_phase(BootPhase::Mbr);
    register_phase(BootPhase::Gpt);
    register_phase(BootPhase::Qfs);
    register_phase(BootPhase::Ext4);
    #[cfg(feature = "hw-i8042")]
    register_phase(BootPhase::I8042);
    #[cfg(feature = "hw-ps2-keyboard")]
    register_phase(BootPhase::Ps2Keyboard);
    #[cfg(feature = "hw-usb-hid")]
    {
        register_phase(BootPhase::Xhci);
        register_phase(BootPhase::UsbCore);
        register_phase(BootPhase::UsbHid);
        register_phase(BootPhase::UsbKeyboard);
    }
    #[cfg(feature = "hw-acpi-ec")]
    register_phase(BootPhase::AcpiEc);
    #[cfg(feature = "hw-laptop-hotkeys")]
    register_phase(BootPhase::EcHotkeys);
    #[cfg(feature = "hw-keyboard")]
    register_phase(BootPhase::Input);
    register_phase(BootPhase::KernelConstructed);
    register_phase(BootPhase::BootInfoApplied);
    register_phase(BootPhase::SupervisorCreated);
    register_phase(BootPhase::RootFs);
    register_phase(BootPhase::Supervisor);
    register_phase(BootPhase::UserspaceLoader);
    register_phase(BootPhase::SpiderRs);
    register_phase(BootPhase::Pid1);
    register_phase(BootPhase::SystemDispatcher);
    register_phase(BootPhase::M1Terminal);
    register_phase(BootPhase::Userspace);
    register_phase(BootPhase::BootRuntime);
    register_phase(BootPhase::Mtss);
    register_phase(BootPhase::BootScreen);
    register_phase(BootPhase::IdleLoop);
    BOOT_PHASE_MANAGER.lock().mark_registered_pending();
}

/// Compatibility shim for older seed-rs code paths; this no longer registers
/// nonexistent services.
pub fn boot_register_default_subsystems() {
    boot_register_compiled_subsystems();
}

fn register_phase(phase: BootPhase) {
    boot_register_subsystem(fallback_descriptor(phase));
}

pub fn boot_register_subsystem(descriptor: SubsystemDescriptor) {
    let duplicate = BOOT_PHASE_MANAGER.lock().register(descriptor);
    if cfg!(feature = "bootdiag-verbose") || cfg!(feature = "bootdiag-serial") {
        if duplicate {
            write_duplicate_registration_serial(descriptor.phase);
        } else {
            write_registration_serial(descriptor.phase);
        }
    }
    // Registration is intentionally silent on the framebuffer in the default
    // fast live mode. Framebuffer milestone painting is debug-only so normal
    // boot transitions do not synchronously repaint the boot UI.
}

pub fn boot_phase_start(phase: BootPhase) {
    crate::kernel::boot_diagnostics::boot_trace_phase_started(phase.name());
    transition(phase, PhaseState::Started, "started");
}

pub fn boot_phase_pending(phase: BootPhase, message: &'static str) {
    transition(phase, PhaseState::Pending, message);
}

pub fn boot_phase_ok(phase: BootPhase) {
    crate::kernel::boot_diagnostics::boot_trace_phase_ok(phase.name());
    transition(phase, PhaseState::Ok, "ok");
}

pub fn boot_phase_online(phase: BootPhase) {
    if phase == BootPhase::Framebuffer {
        crate::kernel::boot_diagnostics::mark_framebuffer_online();
    }
    transition(phase, PhaseState::Online, "online");
}

pub fn boot_phase_enabled(phase: BootPhase) {
    transition(phase, PhaseState::Enabled, "enabled");
}

pub fn boot_phase_failed(phase: BootPhase, message: &'static str) {
    crate::kernel::boot_diagnostics::boot_trace_phase_failed(phase.name(), message);
    transition(phase, PhaseState::Failed, message);
}

pub fn boot_phase_skipped(phase: BootPhase, message: &'static str) {
    transition(phase, PhaseState::Skipped, message);
}

pub fn boot_phase_detected(phase: BootPhase) {
    transition(phase, PhaseState::Detected, "detected");
}

pub fn boot_phase_found(phase: BootPhase) {
    transition(phase, PhaseState::Found, "found");
}

pub fn boot_phase_stub(phase: BootPhase, message: &'static str) {
    transition(phase, PhaseState::Stub, message);
}

pub fn boot_phase_running(phase: BootPhase) {
    transition(phase, PhaseState::Running, "running");
}

pub fn boot_phase_state(phase: BootPhase) -> PhaseState {
    BOOT_PHASE_MANAGER.lock().state(phase)
}

pub fn boot_phase_validate_no_unresolved() {
    {
        let mut manager = BOOT_PHASE_MANAGER.lock();
        manager.validate_no_unresolved();
    }
    if cfg!(feature = "bootdiag-serial") || cfg!(feature = "bootdiag-verbose") {
        write_validation_serial();
    }
    if cfg!(feature = "hw-framebuffer") && framebuffer_ready() {
        boot_phase_render_screen();
    }
}

pub fn boot_phase_current() -> BootPhase {
    BOOT_PHASE_MANAGER.lock().current_phase
}

pub fn boot_phase_progress_percent() -> u8 {
    BOOT_PHASE_MANAGER.lock().progress_percent()
}

pub fn boot_phase_records(mut visit: impl FnMut(&BootPhaseRecord)) {
    BOOT_PHASE_MANAGER
        .lock()
        .for_each_record(|record| visit(record));
}

pub fn boot_phase_snapshot() -> BootPhaseManager {
    *BOOT_PHASE_MANAGER.lock()
}

pub fn boot_phase_render_screen() {
    crate::kernel::boot_screen::render_persistent_boot_screen();
}

fn transition(phase: BootPhase, state: PhaseState, message: &'static str) {
    let registered = crate::arch::x86_64::interrupts::without_interrupts(|| {
        let mut manager = BOOT_PHASE_MANAGER.lock();
        manager.transition(phase, state, message)
    });

    if !registered {
        write_unregistered_transition_serial(phase, state);
        return;
    }
    if cfg!(feature = "bootdiag-serial")
        || cfg!(feature = "bootdiag-verbose")
        || matches!(state, PhaseState::Failed)
    {
        write_transition_serial(phase, state, message);
    }

    if should_render_framebuffer_transition(phase, state) {
        boot_phase_render_screen();
    }
}

fn should_render_framebuffer_transition(phase: BootPhase, state: PhaseState) -> bool {
    framebuffer_milestone_render_enabled()
        && framebuffer_ready()
        && is_framebuffer_repaint_milestone(phase, state)
}

const fn is_framebuffer_repaint_milestone(phase: BootPhase, state: PhaseState) -> bool {
    match (phase, state) {
        // The boot screen and framebuffer phases establish the live UI itself,
        // so repaint them promptly once the framebuffer is available.
        (BootPhase::BootScreen, _) => true,
        (BootPhase::Framebuffer, PhaseState::Ok | PhaseState::Online | PhaseState::Enabled) => true,

        // Failures are user-visible terminal states and should not wait for a
        // later successful milestone before the live UI reflects the fault.
        (_, PhaseState::Failed) => true,

        // Stable user-visible milestones. Stub is included because Mirage uses
        // explicit stub statuses to communicate intentional skeleton coverage.
        (
            _,
            PhaseState::Ok
            | PhaseState::Online
            | PhaseState::Enabled
            | PhaseState::Running
            | PhaseState::Found
            | PhaseState::Stub,
        ) => true,

        // Noisy probe/intermediate states and routine optional skips remain
        // serial-diagnostic events only; they should not thrash the framebuffer.
        (
            _,
            PhaseState::Unregistered
            | PhaseState::Registered
            | PhaseState::Pending
            | PhaseState::Started
            | PhaseState::Detected
            | PhaseState::Skipped,
        ) => false,
    }
}

#[cfg(feature = "hw-framebuffer")]
fn framebuffer_milestone_render_enabled() -> bool {
    true
}

#[cfg(not(feature = "hw-framebuffer"))]
fn framebuffer_milestone_render_enabled() -> bool {
    false
}

fn framebuffer_ready() -> bool {
    matches!(
        boot_phase_state(BootPhase::Framebuffer),
        PhaseState::Ok | PhaseState::Online | PhaseState::Enabled
    )
}

#[allow(unreachable_code)]
fn write_registration_serial(phase: BootPhase) {
    #[cfg(test)]
    {
        let _ = phase;
        return;
    }
    unsafe {
        crate::arch::x86_64::early_debug::com1_write_str("[Phase] ");
        crate::arch::x86_64::early_debug::com1_write_str(phase.name());
        crate::arch::x86_64::early_debug::com1_write_str(": Registered\r\n");
    }
}

#[allow(unreachable_code)]
fn write_duplicate_registration_serial(phase: BootPhase) {
    #[cfg(test)]
    {
        let _ = phase;
        return;
    }
    unsafe {
        crate::arch::x86_64::early_debug::com1_write_str(
            "[Phase] Warning: duplicate registration for ",
        );
        crate::arch::x86_64::early_debug::com1_write_str(phase.name());
        crate::arch::x86_64::early_debug::com1_write_str("\r\n");
    }
}

#[allow(unreachable_code)]
fn write_unregistered_transition_serial(phase: BootPhase, state: PhaseState) {
    // Feature-gated phases are intentionally absent from many builds. Treat
    // transitions for absent phases as silent no-ops so clean boots do not emit
    // noisy "ignored unregistered" diagnostics for drivers that were not compiled in.
    let _ = (phase, state);
}

#[allow(unreachable_code)]
fn write_validation_serial() {
    unsafe {
        crate::arch::x86_64::early_debug::com1_write_str(
            "[Phase] Validation: unresolved registered/pending phases closed\r\n",
        );
    }
}

fn write_transition_serial(phase: BootPhase, state: PhaseState, message: &'static str) {
    unsafe {
        crate::arch::x86_64::early_debug::com1_write_str("[Phase] ");
        crate::arch::x86_64::early_debug::com1_write_str(phase.name());
        crate::arch::x86_64::early_debug::com1_write_str(": ");
        crate::arch::x86_64::early_debug::com1_write_str(state.as_str());
        match state {
            PhaseState::Failed | PhaseState::Skipped | PhaseState::Stub => {
                crate::arch::x86_64::early_debug::com1_write_str(": ");
                crate::arch::x86_64::early_debug::com1_write_str(message);
            }
            _ => {}
        }
        crate::arch::x86_64::early_debug::com1_write_str("\r\n");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_framebuffer_repaint_milestone, BootPhase, BootPhaseManager, PhaseState,
        DEFAULT_SUBSYSTEM_DESCRIPTORS,
    };

    #[test]
    fn registration_makes_records_visible_without_heap() {
        let mut manager = BootPhaseManager::new();
        assert_eq!(manager.state(BootPhase::SeedRs), PhaseState::Unregistered);
        assert!(!manager.register(DEFAULT_SUBSYSTEM_DESCRIPTORS[0]));
        assert_eq!(manager.state(BootPhase::SeedRs), PhaseState::Registered);
        assert!(manager.record(BootPhase::SeedRs).is_some());
    }

    #[test]
    fn duplicate_registration_is_detected_safely() {
        let mut manager = BootPhaseManager::new();
        assert!(!manager.register(DEFAULT_SUBSYSTEM_DESCRIPTORS[0]));
        assert!(manager.register(DEFAULT_SUBSYSTEM_DESCRIPTORS[0]));
    }

    #[test]
    fn unregistered_transition_is_ignored() {
        let mut manager = BootPhaseManager::new();
        assert!(!manager.transition(BootPhase::UsbKeyboard, PhaseState::Started, "started"));
        assert_eq!(
            manager.state(BootPhase::UsbKeyboard),
            PhaseState::Unregistered
        );
        assert!(manager.record(BootPhase::UsbKeyboard).is_none());
    }

    #[test]
    fn phase_state_labels_are_title_case() {
        assert_eq!(PhaseState::Registered.as_str(), "Registered");
        assert_eq!(PhaseState::Pending.as_str(), "Pending");
        assert_eq!(PhaseState::Started.as_str(), "Started");
        assert_eq!(PhaseState::Detected.as_str(), "Detected");
        assert_eq!(PhaseState::Ok.as_str(), "Ok");
        assert_eq!(PhaseState::Online.as_str(), "Online");
        assert_eq!(PhaseState::Enabled.as_str(), "Enabled");
        assert_eq!(PhaseState::Stub.as_str(), "Stub");
        assert_eq!(PhaseState::Skipped.as_str(), "Skipped");
        assert_eq!(PhaseState::Failed.as_str(), "Failed");
        assert_eq!(PhaseState::Running.as_str(), "Running");
    }

    #[test]
    fn progress_uses_registered_weights() {
        let mut manager = BootPhaseManager::new();
        manager.register(DEFAULT_SUBSYSTEM_DESCRIPTORS[0]);
        manager.register(DEFAULT_SUBSYSTEM_DESCRIPTORS[1]);
        manager.transition(BootPhase::SeedRs, PhaseState::Ok, "ok");
        manager.transition(BootPhase::BootInfo, PhaseState::Started, "started");
        assert_eq!(manager.progress_percent(), 80);
    }

    #[test]
    fn validation_closes_unresolved_optional_and_required_phases() {
        let mut manager = BootPhaseManager::new();
        manager.register(DEFAULT_SUBSYSTEM_DESCRIPTORS[0]);
        manager.register(DEFAULT_SUBSYSTEM_DESCRIPTORS[12]);
        manager.mark_registered_pending();

        manager.validate_no_unresolved();

        assert_eq!(manager.state(BootPhase::SeedRs), PhaseState::Failed);
        assert_eq!(manager.state(BootPhase::Framebuffer), PhaseState::Skipped);
    }

    #[test]
    fn framebuffer_repaint_milestones_ignore_intermediate_noise() {
        let noisy_transitions = [
            (BootPhase::KernelMain, PhaseState::Started),
            (BootPhase::RootFs, PhaseState::Pending),
            (BootPhase::Nvme, PhaseState::Detected),
            (BootPhase::Gpt, PhaseState::Detected),
            (BootPhase::Battery, PhaseState::Skipped),
        ];

        let render_count = noisy_transitions
            .iter()
            .filter(|(phase, state)| is_framebuffer_repaint_milestone(*phase, *state))
            .count();

        assert_eq!(
            render_count, 0,
            "intermediate/probe/optional-skip transitions should remain serial-only"
        );
    }

    #[test]
    fn framebuffer_repaint_milestones_count_stable_user_visible_states() {
        let stable_transitions = [
            (BootPhase::KernelMain, PhaseState::Ok),
            (BootPhase::Framebuffer, PhaseState::Online),
            (BootPhase::Input, PhaseState::Enabled),
            (BootPhase::Userspace, PhaseState::Running),
            (BootPhase::SpiderRs, PhaseState::Found),
            (BootPhase::Qfs, PhaseState::Failed),
            (BootPhase::Ext4, PhaseState::Stub),
            (BootPhase::BootScreen, PhaseState::Started),
        ];

        let render_count = stable_transitions
            .iter()
            .filter(|(phase, state)| is_framebuffer_repaint_milestone(*phase, *state))
            .count();

        assert_eq!(
            render_count,
            stable_transitions.len(),
            "stable milestones, Found artifacts, visible stubs, BootScreen, Framebuffer, and failures should repaint"
        );
    }

    #[test]
    fn seed_rs_handoff_registers_defaults_before_seed_transition() {
        let source = include_str!("../arch/x86_64/seed_rs.rs");
        let handoff_start = source
            .find("pub unsafe fn x86_64_handoff() -> !")
            .expect("x86_64_handoff should be present");
        let handoff = &source[handoff_start..];

        let clear_bss = handoff
            .find("boot::clear_bss();")
            .expect("seed handoff should clear BSS before registration");
        let register_compiled = handoff
            .find("boot_register_compiled_subsystems();")
            .expect("seed handoff should register compiled boot phases");
        let start_seed = handoff
            .find("boot_phase_start(BootPhase::SeedRs);")
            .expect("seed handoff should start the Seed-rs phase");
        let first_marker = handoff
            .find("[seed-rs 01] entered seed entry")
            .expect("seed handoff should emit its first diagnostic after phase start");

        assert!(
            clear_bss < register_compiled,
            "BSS must be cleared before writing the boot phase manager's static table"
        );
        assert!(
            register_compiled < start_seed,
            "compiled registration must precede Seed-rs start to avoid auto-registration warnings"
        );
        assert!(
            start_seed < first_marker,
            "Seed-rs diagnostics should be emitted after the phase transition is tracked"
        );
    }
}
