//! Mirage-dispatch-rs: fixed-capacity kernel startup orchestration.
//!
//! The dispatcher is intentionally small and `no_std`: compiled services register
//! static service objects, the dispatcher validates feature gates and dependencies,
//! probes runtime presence, and only then emits `Started` before calling `start()`.
//! The Boot Phase Manager remains the state-reporting sink; services do not mutate
//! phase state directly when they are run through this orchestrator.

#[cfg(not(test))]
use crate::kernel::boot_phase::{boot_register_subsystem, SubsystemDescriptor};

use crate::kernel::boot_phase::{
    boot_phase_detected, boot_phase_enabled, boot_phase_failed, boot_phase_ok, boot_phase_online,
    boot_phase_skipped, boot_phase_start, boot_phase_stub, BootPhase, PhaseState,
    SubsystemCategory, BOOT_PHASE_COUNT,
};

pub const DISPATCH_REGISTRY_CAPACITY: usize = BOOT_PHASE_COUNT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchCategory {
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

impl DispatchCategory {
    pub const fn as_subsystem_category(self) -> SubsystemCategory {
        match self {
            Self::Seed => SubsystemCategory::Seed,
            Self::Boot => SubsystemCategory::Boot,
            Self::Architecture => SubsystemCategory::Architecture,
            Self::Memory => SubsystemCategory::Memory,
            Self::Device => SubsystemCategory::Device,
            Self::Input => SubsystemCategory::Input,
            Self::Storage => SubsystemCategory::Storage,
            Self::Supervisor => SubsystemCategory::Supervisor,
            Self::Userspace => SubsystemCategory::Userspace,
            Self::Scheduler => SubsystemCategory::Scheduler,
            Self::Debug => SubsystemCategory::Debug,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DispatchDescriptor {
    pub name: &'static str,
    pub phase: BootPhase,
    pub category: DispatchCategory,
    pub required: bool,
    pub feature_gate: Option<&'static str>,
    pub dependencies: &'static [BootPhase],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchProbeResult {
    Present,
    NotPresent(&'static str),
    Unsupported(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchResult {
    Ok,
    Online,
    Enabled,
    Skipped(&'static str),
    Stub(&'static str),
    Failed(&'static str),
}

pub trait DispatchService: Sync {
    fn descriptor(&self) -> DispatchDescriptor;
    fn probe(&self) -> DispatchProbeResult;
    fn start(&self) -> DispatchResult;
    fn stop(&self) -> DispatchResult;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchRegisterError {
    RegistryFull,
    Duplicate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DispatchReport {
    pub registered: usize,
    pub dispatched: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl DispatchReport {
    pub const fn new() -> Self {
        Self {
            registered: 0,
            dispatched: 0,
            skipped: 0,
            failed: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct DispatchRegistry {
    services: [Option<&'static dyn DispatchService>; DISPATCH_REGISTRY_CAPACITY],
    phase_index: [Option<usize>; BOOT_PHASE_COUNT],
    len: usize,
}

impl DispatchRegistry {
    pub const fn new() -> Self {
        Self {
            services: [None; DISPATCH_REGISTRY_CAPACITY],
            phase_index: [None; BOOT_PHASE_COUNT],
            len: 0,
        }
    }

    pub fn register(
        &mut self,
        service: &'static dyn DispatchService,
    ) -> Result<bool, DispatchRegisterError> {
        let descriptor = service.descriptor();
        let phase_slot = descriptor.phase as usize;
        if self.phase_index[phase_slot].is_some() || self.find_by_name(descriptor.name).is_some() {
            write_dispatch_duplicate(descriptor.name);
            return Ok(false);
        }
        if self.len >= DISPATCH_REGISTRY_CAPACITY {
            write_dispatch_full(descriptor.name);
            return Err(DispatchRegisterError::RegistryFull);
        }

        self.services[self.len] = Some(service);
        self.phase_index[phase_slot] = Some(self.len);
        self.len += 1;
        #[cfg(not(test))]
        boot_register_subsystem(dispatch_subsystem_descriptor(descriptor));
        write_dispatch_registered(descriptor.name);
        Ok(true)
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn lookup(&self, phase: BootPhase) -> Option<&'static dyn DispatchService> {
        match self.phase_index[phase as usize] {
            Some(index) => self.services[index],
            None => None,
        }
    }

    pub fn dispatch_all(&self) -> DispatchReport {
        let mut report = DispatchReport::new();
        report.registered = self.len;
        let mut index = 0usize;
        while index < self.len {
            if let Some(service) = self.services[index] {
                dispatch_one(service, &mut report);
            }
            index += 1;
        }
        report
    }

    fn find_by_name(&self, name: &'static str) -> Option<usize> {
        let mut index = 0usize;
        while index < self.len {
            if let Some(service) = self.services[index] {
                if service.descriptor().name.as_ptr() == name.as_ptr()
                    && service.descriptor().name.len() == name.len()
                {
                    return Some(index);
                }
            }
            index += 1;
        }
        None
    }
}

fn dispatch_one(service: &'static dyn DispatchService, report: &mut DispatchReport) {
    let descriptor = service.descriptor();

    if let Some(gate) = descriptor.feature_gate {
        if !feature_enabled(gate) {
            report.skipped += 1;
            write_dispatch_skipped(descriptor.name, "feature gate disabled");
            boot_phase_skipped(descriptor.phase, "feature gate disabled");
            return;
        }
    }

    if let Some(message) = failed_dependency(descriptor.dependencies) {
        if descriptor.required {
            report.failed += 1;
            write_dispatch_failed(descriptor.name, message);
            boot_phase_failed(descriptor.phase, message);
        } else {
            report.skipped += 1;
            write_dispatch_skipped(descriptor.name, message);
            boot_phase_skipped(descriptor.phase, message);
        }
        return;
    }

    match service.probe() {
        DispatchProbeResult::Present => {
            boot_phase_detected(descriptor.phase);
        }
        DispatchProbeResult::NotPresent(message) | DispatchProbeResult::Unsupported(message) => {
            report.skipped += 1;
            write_dispatch_skipped(descriptor.name, message);
            boot_phase_skipped(descriptor.phase, message);
            return;
        }
    }

    write_dispatch_dispatching(descriptor.name);
    boot_phase_start(descriptor.phase);
    report.dispatched += 1;
    match service.start() {
        DispatchResult::Ok => boot_phase_ok(descriptor.phase),
        DispatchResult::Online => boot_phase_online(descriptor.phase),
        DispatchResult::Enabled => boot_phase_enabled(descriptor.phase),
        DispatchResult::Skipped(message) => {
            report.skipped += 1;
            write_dispatch_skipped(descriptor.name, message);
            boot_phase_skipped(descriptor.phase, message);
        }
        DispatchResult::Stub(message) => boot_phase_stub(descriptor.phase, message),
        DispatchResult::Failed(message) => {
            report.failed += 1;
            write_dispatch_failed(descriptor.name, message);
            boot_phase_failed(descriptor.phase, message);
        }
    }
}

fn failed_dependency(dependencies: &'static [BootPhase]) -> Option<&'static str> {
    let mut index = 0usize;
    while index < dependencies.len() {
        match crate::kernel::boot_phase::boot_phase_state(dependencies[index]) {
            PhaseState::Ok
            | PhaseState::Ready
            | PhaseState::Online
            | PhaseState::Enabled
            | PhaseState::Detected
            | PhaseState::Stub
            | PhaseState::Degraded
            | PhaseState::Running => {}
            PhaseState::Failed => return Some("dependency failed"),
            PhaseState::Skipped | PhaseState::Disabled => return Some("dependency skipped"),
            _ => return Some("dependency not ready"),
        }
        index += 1;
    }
    None
}

#[cfg(not(test))]
const fn dispatch_subsystem_descriptor(descriptor: DispatchDescriptor) -> SubsystemDescriptor {
    SubsystemDescriptor {
        phase: descriptor.phase,
        name: descriptor.name,
        category: descriptor.category.as_subsystem_category(),
        required: descriptor.required,
        weight: 3,
    }
}

pub const fn feature_enabled(gate: &'static str) -> bool {
    match gate.as_bytes() {
        b"hw-pci" => cfg!(feature = "hw-pci"),
        b"hw-acpi" => cfg!(feature = "hw-acpi"),
        b"hw-amd64" => cfg!(feature = "hw-amd64"),
        b"hw-ryzen" => cfg!(feature = "hw-ryzen"),
        b"hw-amd-chipset" => cfg!(feature = "hw-amd-chipset"),
        b"hw-amd-iommu" => cfg!(feature = "hw-amd-iommu"),
        b"hw-nvme" => cfg!(feature = "hw-nvme"),
        b"hw-ahci" => cfg!(feature = "hw-ahci"),
        b"hw-xhci" => cfg!(feature = "hw-xhci"),
        b"hw-i8042" => cfg!(feature = "hw-i8042"),
        b"hw-ps2-keyboard" => cfg!(feature = "hw-ps2-keyboard"),
        b"hw-usb-hid" => cfg!(feature = "hw-usb-hid"),
        b"hw-acpi-ec" => cfg!(feature = "hw-acpi-ec"),
        b"hw-laptop-hotkeys" => cfg!(feature = "hw-laptop-hotkeys"),
        b"hw-keyboard" => cfg!(feature = "hw-keyboard"),
        b"hw-framebuffer" => cfg!(feature = "hw-framebuffer"),
        b"hw-amdgpu" => cfg!(feature = "hw-amdgpu"),
        _ => false,
    }
}

fn write_dispatch_registered(name: &'static str) {
    write_dispatch_line("registered", name, "");
}
fn write_dispatch_dispatching(name: &'static str) {
    write_dispatch_line("dispatching", name, "");
}
fn write_dispatch_duplicate(name: &'static str) {
    write_dispatch_line("duplicate", name, "");
}
fn write_dispatch_full(name: &'static str) {
    write_dispatch_line("registry full", name, "");
}
fn write_dispatch_skipped(name: &'static str, reason: &'static str) {
    write_dispatch_line("skipped", name, reason);
}
fn write_dispatch_failed(name: &'static str, reason: &'static str) {
    write_dispatch_line("failed", name, reason);
}

#[allow(unreachable_code)]
fn write_dispatch_line(action: &'static str, name: &'static str, reason: &'static str) {
    #[cfg(test)]
    {
        let _ = (action, name, reason);
        return;
    }
    unsafe {
        crate::arch::x86_64::early_debug::com1_write_str("[Dispatch] ");
        crate::arch::x86_64::early_debug::com1_write_str(action);
        crate::arch::x86_64::early_debug::com1_write_str(": ");
        crate::arch::x86_64::early_debug::com1_write_str(name);
        crate::arch::x86_64::early_debug::com1_write_str("\r\n");
        if !reason.is_empty() {
            crate::arch::x86_64::early_debug::com1_write_str("reason: ");
            crate::arch::x86_64::early_debug::com1_write_str(reason);
            crate::arch::x86_64::early_debug::com1_write_str("\r\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestService;
    impl DispatchService for TestService {
        fn descriptor(&self) -> DispatchDescriptor {
            DispatchDescriptor {
                name: "test",
                phase: BootPhase::BootScreen,
                category: DispatchCategory::Debug,
                required: false,
                feature_gate: None,
                dependencies: &[],
            }
        }
        fn probe(&self) -> DispatchProbeResult {
            DispatchProbeResult::NotPresent("no hardware")
        }
        fn start(&self) -> DispatchResult {
            DispatchResult::Online
        }
        fn stop(&self) -> DispatchResult {
            DispatchResult::Ok
        }
    }
    static TEST_SERVICE: TestService = TestService;

    #[test]
    fn registry_is_fixed_capacity_and_idempotent() {
        let mut registry = DispatchRegistry::new();
        assert_eq!(registry.register(&TEST_SERVICE), Ok(true));
        assert_eq!(registry.register(&TEST_SERVICE), Ok(false));
        assert_eq!(registry.len(), 1);
        assert!(registry.lookup(BootPhase::BootScreen).is_some());
    }
}
