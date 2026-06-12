//! Canonical no-heap boot subsystem registration and status tracking.
//!
//! Mirage's early boot path uses this module as the single source of truth for
//! subsystem visibility.  Every subsystem registers a static descriptor before
//! it starts, then reports state transitions through the functions below.  The
//! table is fixed-size, `no_std` friendly, and queryable by the framebuffer boot
//! screen and future debug shell without requiring heap allocation.

use crate::kernel::sync::SpinLock;

/// Maximum number of boot subsystem records tracked without allocation.
pub const BOOT_PHASE_CAPACITY: usize = 46;
/// Current number of canonical Mirage boot phases.
pub const BOOT_PHASE_COUNT: usize = BOOT_PHASE_CAPACITY;

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
    Nvme,
    Ahci,
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
    Userspace,
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
            Self::Nvme => "NVMe",
            Self::Ahci => "AHCI",
            Self::UsbCore => "USB Core",
            Self::UsbHid => "USB HID",
            Self::UsbKeyboard => "USB Keyboard",
            Self::KernelMapper => "Kernel Mapper",
            Self::RootFs => "Root Filesystem",
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
    Ok,
    Online,
    Enabled,
    Failed,
    Skipped,
    Detected,
    Stub,
}

impl PhaseState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unregistered => "UNREGISTERED",
            Self::Registered => "REGISTERED",
            Self::Pending => "PENDING",
            Self::Started => "STARTED",
            Self::Ok => "OK",
            Self::Online => "ONLINE",
            Self::Enabled => "ENABLED",
            Self::Failed => "FAILED",
            Self::Skipped => "SKIPPED",
            Self::Detected => "DETECTED",
            Self::Stub => "STUB",
        }
    }

    const fn weighted_progress(self, required: bool, weight: u8) -> u16 {
        let weight = weight as u16;
        match self {
            Self::Ok | Self::Online | Self::Enabled => weight,
            Self::Skipped | Self::Detected | Self::Stub => {
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

    pub fn transition(&mut self, phase: BootPhase, state: PhaseState, message: &'static str) {
        self.current_phase = phase;
        let index = phase.index();
        if self.records[index].state == PhaseState::Unregistered {
            self.records[index] = BootPhaseRecord {
                descriptor: fallback_descriptor(phase),
                state: PhaseState::Registered,
                message: "auto-registered",
            };
        }
        self.records[index].state = state;
        self.records[index].message = message;
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
        BootPhase::I8042,
        "I8042",
        SubsystemCategory::Input,
        false,
        2,
    ),
    descriptor(
        BootPhase::Ps2Keyboard,
        "PS/2 Kbd",
        SubsystemCategory::Input,
        false,
        3,
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
        BootPhase::Nvme,
        "NVMe",
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
    descriptor(BootPhase::Xhci, "xHCI", SubsystemCategory::Device, false, 4),
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
        "USB Kbd",
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
        BootPhase::Userspace,
        "Userspace",
        SubsystemCategory::Userspace,
        false,
        3,
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

/// Register all Milestone 1.1 core subsystems and leave them pending.
pub fn boot_register_default_subsystems() {
    let mut index = 0usize;
    while index < DEFAULT_SUBSYSTEM_DESCRIPTORS.len() {
        boot_register_subsystem(DEFAULT_SUBSYSTEM_DESCRIPTORS[index]);
        index += 1;
    }
    BOOT_PHASE_MANAGER.lock().mark_registered_pending();
}

pub fn boot_register_subsystem(descriptor: SubsystemDescriptor) {
    let duplicate = BOOT_PHASE_MANAGER.lock().register(descriptor);
    if duplicate {
        write_duplicate_registration_serial(descriptor.phase);
    } else {
        write_registration_serial(descriptor.phase);
    }
    if framebuffer_ready() {
        boot_phase_render_screen();
    }
}

pub fn boot_phase_start(phase: BootPhase) {
    transition(phase, PhaseState::Started, "started");
}

pub fn boot_phase_ok(phase: BootPhase) {
    transition(phase, PhaseState::Ok, "ok");
}

pub fn boot_phase_online(phase: BootPhase) {
    transition(phase, PhaseState::Online, "online");
}

pub fn boot_phase_enabled(phase: BootPhase) {
    transition(phase, PhaseState::Enabled, "enabled");
}

pub fn boot_phase_failed(phase: BootPhase, message: &'static str) {
    transition(phase, PhaseState::Failed, message);
}

pub fn boot_phase_skipped(phase: BootPhase, message: &'static str) {
    transition(phase, PhaseState::Skipped, message);
}

pub fn boot_phase_detected(phase: BootPhase) {
    transition(phase, PhaseState::Detected, "detected");
}

pub fn boot_phase_stub(phase: BootPhase, message: &'static str) {
    transition(phase, PhaseState::Stub, message);
}

pub fn boot_phase_state(phase: BootPhase) -> PhaseState {
    BOOT_PHASE_MANAGER.lock().state(phase)
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
    let auto_registered = {
        let mut manager = BOOT_PHASE_MANAGER.lock();
        let was_unregistered = manager.state(phase) == PhaseState::Unregistered;
        manager.transition(phase, state, message);
        was_unregistered
    };

    if auto_registered {
        write_auto_registration_warning_serial(phase);
    }
    write_transition_serial(phase, state, message);

    if framebuffer_ready() {
        boot_phase_render_screen();
    }
}

fn framebuffer_ready() -> bool {
    matches!(
        boot_phase_state(BootPhase::Framebuffer),
        PhaseState::Ok | PhaseState::Online | PhaseState::Enabled
    )
}

fn write_registration_serial(phase: BootPhase) {
    unsafe {
        crate::arch::x86_64::early_debug::com1_write_str("[phase] ");
        crate::arch::x86_64::early_debug::com1_write_str(phase.name());
        crate::arch::x86_64::early_debug::com1_write_str(": registered\r\n");
    }
}

fn write_duplicate_registration_serial(phase: BootPhase) {
    unsafe {
        crate::arch::x86_64::early_debug::com1_write_str(
            "[phase] WARNING: duplicate registration for ",
        );
        crate::arch::x86_64::early_debug::com1_write_str(phase.name());
        crate::arch::x86_64::early_debug::com1_write_str("\r\n");
    }
}

fn write_auto_registration_warning_serial(phase: BootPhase) {
    unsafe {
        crate::arch::x86_64::early_debug::com1_write_str("[phase] WARNING: ");
        crate::arch::x86_64::early_debug::com1_write_str(phase.name());
        crate::arch::x86_64::early_debug::com1_write_str(" started without registration\r\n");
    }
}

fn write_transition_serial(phase: BootPhase, state: PhaseState, message: &'static str) {
    unsafe {
        crate::arch::x86_64::early_debug::com1_write_str("[phase] ");
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
    use super::{BootPhase, BootPhaseManager, PhaseState, DEFAULT_SUBSYSTEM_DESCRIPTORS};

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
    fn progress_uses_registered_weights() {
        let mut manager = BootPhaseManager::new();
        manager.register(DEFAULT_SUBSYSTEM_DESCRIPTORS[0]);
        manager.register(DEFAULT_SUBSYSTEM_DESCRIPTORS[1]);
        manager.transition(BootPhase::SeedRs, PhaseState::Ok, "ok");
        manager.transition(BootPhase::BootInfo, PhaseState::Started, "started");
        assert_eq!(manager.progress_percent(), 80);
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
        let register_defaults = handoff
            .find("boot_register_default_subsystems();")
            .expect("seed handoff should register default boot phases");
        let start_seed = handoff
            .find("boot_phase_start(BootPhase::SeedRs);")
            .expect("seed handoff should start the Seed-rs phase");
        let first_marker = handoff
            .find("[seed-rs 01] entered seed entry")
            .expect("seed handoff should emit its first diagnostic after phase start");

        assert!(
            clear_bss < register_defaults,
            "BSS must be cleared before writing the boot phase manager's static table"
        );
        assert!(
            register_defaults < start_seed,
            "default registration must precede Seed-rs start to avoid auto-registration warnings"
        );
        assert!(
            start_seed < first_marker,
            "Seed-rs diagnostics should be emitted after the phase transition is tracked"
        );
    }
}
