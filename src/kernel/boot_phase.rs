//! Formal no-heap boot phase tracking for early Mirage startup.
//!
//! The phase manager is intentionally static and allocation-free so seed-rs,
//! x86_64 architecture setup, and early kernel policy code can report progress
//! before the heap is online. Serial output is emitted with raw COM1 routines;
//! framebuffer rendering is best-effort and begins only after the framebuffer
//! phase has reached `Ok`.

use crate::kernel::sync::SpinLock;

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
    PhysicalAllocator,
    KernelMapper,
    Heap,
    MemoryMap,
    Memory,
    Framebuffer,
    Idt,
    Pic,
    Interrupts,
    KernelConstructed,
    BootInfoApplied,
    SupervisorCreated,
    RootFs,
    Supervisor,
    Userspace,
    Mtss,
    I8042,
    Ps2Keyboard,
    Xhci,
    UsbHidKeyboard,
    AcpiEc,
    AcpiEcHotkeys,
    InputSubsystem,
    BootScreen,
    IdleLoop,
}

impl BootPhase {
    pub const fn name(self) -> &'static str {
        match self {
            Self::SeedRs => "Seed-rs",
            Self::BootInfo => "BootInfo",
            Self::KernelMain => "KernelMain",
            Self::Architecture => "Architecture",
            Self::Serial => "Serial",
            Self::Gdt => "GDT",
            Self::PhysicalAllocator => "PhysicalAllocator",
            Self::KernelMapper => "KernelMapper",
            Self::Heap => "Heap",
            Self::MemoryMap => "MemoryMap",
            Self::Memory => "Memory",
            Self::Framebuffer => "Framebuffer",
            Self::Idt => "IDT",
            Self::Pic => "PIC",
            Self::Interrupts => "Interrupts",
            Self::KernelConstructed => "KernelConstructed",
            Self::BootInfoApplied => "BootInfoApplied",
            Self::SupervisorCreated => "SupervisorCreated",
            Self::RootFs => "Root FS",
            Self::Supervisor => "Supervisor",
            Self::Userspace => "Userspace",
            Self::Mtss => "MTSS",
            Self::I8042 => "I8042",
            Self::Ps2Keyboard => "PS/2 Kbd",
            Self::Xhci => "xHCI",
            Self::UsbHidKeyboard => "USB Kbd",
            Self::AcpiEc => "ACPI EC",
            Self::AcpiEcHotkeys => "EC Hotkeys",
            Self::InputSubsystem => "Input",
            Self::BootScreen => "BootScreen",
            Self::IdleLoop => "IdleLoop",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// State of a tracked boot phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhaseState {
    Pending,
    Started,
    Ok,
    Failed,
    Skipped,
    Stub,
}

impl PhaseState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Started => "STARTED",
            Self::Ok => "OK",
            Self::Failed => "FAILED",
            Self::Skipped => "SKIPPED",
            Self::Stub => "STUB",
        }
    }

    const fn progress_units(self) -> u16 {
        match self {
            Self::Ok | Self::Skipped | Self::Stub => 2,
            Self::Started => 1,
            Self::Pending | Self::Failed => 0,
        }
    }
}

/// Fixed record for a boot phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootPhaseRecord {
    pub phase: BootPhase,
    pub state: PhaseState,
    pub marker: &'static str,
    pub message: &'static str,
}

/// Number of fixed entries in the boot phase table.
pub const BOOT_PHASE_COUNT: usize = 31;

/// No-heap boot phase manager with a fixed static phase table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootPhaseManager {
    pub records: [BootPhaseRecord; BOOT_PHASE_COUNT],
    pub current_phase: BootPhase,
}

impl BootPhaseManager {
    pub const fn new() -> Self {
        Self {
            records: [
                record(BootPhase::SeedRs, "SEED"),
                record(BootPhase::BootInfo, "BOOTINFO"),
                record(BootPhase::KernelMain, "KMAIN"),
                record(BootPhase::Architecture, "ARCH"),
                record(BootPhase::Serial, "SERIAL"),
                record(BootPhase::Gdt, "GDT"),
                record(BootPhase::PhysicalAllocator, "PALLOC"),
                record(BootPhase::KernelMapper, "KMAP"),
                record(BootPhase::Heap, "HEAP"),
                record(BootPhase::MemoryMap, "MMAP"),
                record(BootPhase::Memory, "MEM"),
                record(BootPhase::Framebuffer, "FB"),
                record(BootPhase::Idt, "IDT"),
                record(BootPhase::Pic, "PIC"),
                record(BootPhase::Interrupts, "IRQ"),
                record(BootPhase::KernelConstructed, "KERNEL"),
                record(BootPhase::BootInfoApplied, "BIAPPLY"),
                record(BootPhase::SupervisorCreated, "SUPNEW"),
                record(BootPhase::RootFs, "ROOTFS"),
                record(BootPhase::Supervisor, "SUP"),
                record(BootPhase::Userspace, "USER"),
                record(BootPhase::Mtss, "MTSS"),
                record(BootPhase::I8042, "I8042"),
                record(BootPhase::Ps2Keyboard, "PS2KBD"),
                record(BootPhase::Xhci, "XHCI"),
                record(BootPhase::UsbHidKeyboard, "USBKBD"),
                record(BootPhase::AcpiEc, "ACPIEC"),
                record(BootPhase::AcpiEcHotkeys, "ECHOTKEY"),
                record(BootPhase::InputSubsystem, "INPUT"),
                record(BootPhase::BootScreen, "SCREEN"),
                record(BootPhase::IdleLoop, "IDLE"),
            ],
            current_phase: BootPhase::SeedRs,
        }
    }

    pub fn transition(&mut self, phase: BootPhase, state: PhaseState, message: &'static str) {
        self.current_phase = phase;
        let index = phase.index();
        self.records[index].state = state;
        self.records[index].message = message;
    }

    pub const fn state(&self, phase: BootPhase) -> PhaseState {
        self.records[phase.index()].state
    }

    pub fn progress_percent(&self) -> u8 {
        let mut index = 0usize;
        let mut units = 0u16;
        while index < BOOT_PHASE_COUNT {
            units += self.records[index].state.progress_units();
            index += 1;
        }
        ((units * 100) / ((BOOT_PHASE_COUNT as u16) * 2)) as u8
    }

    pub fn has_failed(&self) -> bool {
        let mut index = 0usize;
        while index < BOOT_PHASE_COUNT {
            if self.records[index].state == PhaseState::Failed {
                return true;
            }
            index += 1;
        }
        false
    }
}

const fn record(phase: BootPhase, marker: &'static str) -> BootPhaseRecord {
    BootPhaseRecord {
        phase,
        state: PhaseState::Pending,
        marker,
        message: "pending",
    }
}

static BOOT_PHASE_MANAGER: SpinLock<BootPhaseManager> = SpinLock::new(BootPhaseManager::new());

pub fn boot_phase_start(phase: BootPhase) {
    transition(phase, PhaseState::Started, "started");
}

pub fn boot_phase_ok(phase: BootPhase) {
    transition(phase, PhaseState::Ok, "ok");
}

pub fn boot_phase_failed(phase: BootPhase, message: &'static str) {
    transition(phase, PhaseState::Failed, message);
}

pub fn boot_phase_skipped(phase: BootPhase, message: &'static str) {
    transition(phase, PhaseState::Skipped, message);
}

pub fn boot_phase_stub(phase: BootPhase, message: &'static str) {
    transition(phase, PhaseState::Stub, message);
}

pub fn boot_phase_current() -> BootPhase {
    BOOT_PHASE_MANAGER.lock().current_phase
}

pub fn boot_phase_state(phase: BootPhase) -> PhaseState {
    BOOT_PHASE_MANAGER.lock().state(phase)
}

pub fn boot_phase_progress_percent() -> u8 {
    BOOT_PHASE_MANAGER.lock().progress_percent()
}

pub fn boot_phase_snapshot() -> BootPhaseManager {
    *BOOT_PHASE_MANAGER.lock()
}

pub fn boot_phase_render_screen() {
    crate::kernel::boot_screen::render_persistent_boot_screen();
}

fn transition(phase: BootPhase, state: PhaseState, message: &'static str) {
    {
        let mut manager = BOOT_PHASE_MANAGER.lock();
        manager.transition(phase, state, message);
    }

    write_transition_serial(phase, state, message);

    if framebuffer_ready() {
        boot_phase_render_screen();
    }
}

fn framebuffer_ready() -> bool {
    let state = boot_phase_state(BootPhase::Framebuffer);
    state == PhaseState::Ok
}

fn write_transition_serial(phase: BootPhase, state: PhaseState, message: &'static str) {
    unsafe {
        crate::arch::x86_64::early_debug::com1_write_str("[phase] ");
        crate::arch::x86_64::early_debug::com1_write_str(phase.name());
        crate::arch::x86_64::early_debug::com1_write_str(": ");
        match state {
            PhaseState::Started => crate::arch::x86_64::early_debug::com1_write_str("started"),
            PhaseState::Ok => crate::arch::x86_64::early_debug::com1_write_str("ok"),
            PhaseState::Failed => {
                crate::arch::x86_64::early_debug::com1_write_str("failed: ");
                crate::arch::x86_64::early_debug::com1_write_str(message);
            }
            PhaseState::Skipped => {
                crate::arch::x86_64::early_debug::com1_write_str("skipped: ");
                crate::arch::x86_64::early_debug::com1_write_str(message);
            }
            PhaseState::Stub => {
                crate::arch::x86_64::early_debug::com1_write_str("stub: ");
                crate::arch::x86_64::early_debug::com1_write_str(message);
            }
            PhaseState::Pending => crate::arch::x86_64::early_debug::com1_write_str("pending"),
        }
        crate::arch::x86_64::early_debug::com1_write_str("\r\n");
    }
}
