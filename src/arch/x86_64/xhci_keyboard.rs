//! Modular hardware-backed USB driver stack for early Mirage input.
//!
//! This file keeps the current x86_64 boot path no-heap and bounded while
//! splitting USB into kernel-registered modules: `xhci-host0`, `usb-core0`,
//! `usb-hid0`, and `usb-kbd0`.  The boundaries mirror the future supervised
//! driver-service ownership model and avoid the old fragile inline HID init path.

use crate::arch::x86_64::io::{inb, outb};
use crate::kernel::device::{DeviceDriver, DeviceError, DeviceKind};
use crate::kernel::input::{
    copy_mirage_events, mark_source_online, publish_keyboard_event, InputRawSource, KeyCode,
    KeyModifiers, KeyState, KeyboardEvent,
};
use crate::subkernel::{DeviceSecurity, SecurityClass};

const PCI_CONFIG_ADDRESS: u16 = 0xcf8;
const PCI_CONFIG_DATA: u16 = 0xcfc;
const PCI_CLASS_SERIAL_BUS: u8 = 0x0c;
const PCI_SUBCLASS_USB: u8 = 0x03;
const PCI_PROGIF_XHCI: u8 = 0x30;

const USBSTS_HCH: u32 = 1 << 0;
const USBCMD_RUN: u32 = 1 << 0;
const USBCMD_RESET: u32 = 1 << 1;
const PORTSC_CCS: u32 = 1 << 0;
const PORTSC_PED: u32 = 1 << 1;
const PORTSC_PR: u32 = 1 << 4;
const PORTSC_PP: u32 = 1 << 9;
const PORT_REGISTER_STRIDE: usize = 0x10;
const PORT_REGISTER_BASE: usize = 0x400;
const WAIT_LIMIT: usize = 1_000_000;
const MAX_USB_DEVICES: usize = 8;
const MAX_HID_DEVICES: usize = 4;

const XHCI_MODULE: usize = 0;
const USB_CORE_MODULE: usize = 1;
const USB_HID_MODULE: usize = 2;
const USB_KBD_MODULE: usize = 3;
const DRIVER_MODULE_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverCategory {
    Bus,
    HostController,
    UsbClass,
    Input,
    Storage,
    Network,
    Display,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverStatus {
    Registered,
    Initialized,
    Online,
    Skipped,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleInitStatus {
    Online,
    Ok,
    Skipped(&'static str),
    Failed(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverModuleDescriptor {
    pub name: &'static str,
    pub category: DriverCategory,
    pub phase: crate::kernel::boot_phase::BootPhase,
    pub required: bool,
    pub dependencies: &'static [crate::kernel::boot_phase::BootPhase],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverError {
    DependencySkipped(&'static str),
    DependencyFailed(&'static str),
    NoController,
    NoDevice,
    NoHid,
    NoKeyboard,
    InvalidMmio(&'static str),
    Timeout(&'static str),
    DescriptorMalformed,
    EndpointSetupFailed,
}

impl DriverError {
    const fn message(self) -> &'static str {
        match self {
            Self::DependencySkipped(name) => name,
            Self::DependencyFailed(name) => name,
            Self::NoController => "no xHCI controller",
            Self::NoDevice => "no USB devices",
            Self::NoHid => "no HID devices",
            Self::NoKeyboard => "no HID boot keyboard",
            Self::InvalidMmio(stage) => stage,
            Self::Timeout(stage) => stage,
            Self::DescriptorMalformed => "descriptor malformed",
            Self::EndpointSetupFailed => "endpoint setup failed",
        }
    }
}

pub trait DriverModule {
    fn descriptor(&self) -> DriverModuleDescriptor;
    fn init(&self) -> Result<(), DriverError>;
    fn start(&self) -> Result<(), DriverError>;
    fn stop(&self) -> Result<(), DriverError>;
    fn poll(&self);
    fn status(&self) -> DriverStatus;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverRegistry {
    modules: [Option<DriverModuleDescriptor>; DRIVER_MODULE_CAPACITY],
    statuses: [DriverStatus; DRIVER_MODULE_CAPACITY],
}

impl DriverRegistry {
    pub const fn new() -> Self {
        Self {
            modules: [None; DRIVER_MODULE_CAPACITY],
            statuses: [DriverStatus::Registered; DRIVER_MODULE_CAPACITY],
        }
    }

    fn register(&mut self, slot: usize, descriptor: DriverModuleDescriptor) {
        if slot < DRIVER_MODULE_CAPACITY {
            self.modules[slot] = Some(descriptor);
            self.statuses[slot] = DriverStatus::Registered;
        }
    }

    fn set_status(&mut self, slot: usize, status: DriverStatus) {
        if slot < DRIVER_MODULE_CAPACITY {
            self.statuses[slot] = status;
        }
    }

    const fn status(&self, slot: usize) -> DriverStatus {
        self.statuses[slot]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct XhciController {
    function: PciFunction,
    mmio_base: usize,
    cap: usize,
    op: usize,
    runtime: usize,
    doorbells: usize,
    max_ports: u8,
    max_slots: u8,
    context_size: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UsbDeviceRecord {
    id: u8,
    port: u8,
    endpoint_count: u8,
    is_hid_boot_keyboard_candidate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HidDeviceRecord {
    device_id: u8,
    interface_number: u8,
    interrupt_in_endpoint: u8,
    max_packet_size: u16,
    boot_keyboard: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UsbDriverStackState {
    registry: DriverRegistry,
    xhci: Option<XhciController>,
    devices: [Option<UsbDeviceRecord>; MAX_USB_DEVICES],
    hids: [Option<HidDeviceRecord>; MAX_HID_DEVICES],
    keyboard_online: bool,
}

impl UsbDriverStackState {
    pub const fn new() -> Self {
        Self {
            registry: DriverRegistry::new(),
            xhci: None,
            devices: [None; MAX_USB_DEVICES],
            hids: [None; MAX_HID_DEVICES],
            keyboard_online: false,
        }
    }

    fn clear_runtime(&mut self) {
        self.xhci = None;
        self.devices = [None; MAX_USB_DEVICES];
        self.hids = [None; MAX_HID_DEVICES];
        self.keyboard_online = false;
    }

    fn add_device(&mut self, record: UsbDeviceRecord) -> bool {
        let mut index = 0usize;
        while index < self.devices.len() {
            if self.devices[index].is_none() {
                self.devices[index] = Some(record);
                return true;
            }
            index += 1;
        }
        false
    }

    fn add_hid(&mut self, record: HidDeviceRecord) -> bool {
        let mut index = 0usize;
        while index < self.hids.len() {
            if self.hids[index].is_none() {
                self.hids[index] = Some(record);
                return true;
            }
            index += 1;
        }
        false
    }

    fn hid_count(&self) -> usize {
        let mut count = 0usize;
        let mut index = 0usize;
        while index < self.hids.len() {
            if self.hids[index].is_some() {
                count += 1;
            }
            index += 1;
        }
        count
    }

    fn has_boot_keyboard(&self) -> bool {
        let mut index = 0usize;
        while index < self.hids.len() {
            if let Some(record) = self.hids[index] {
                if record.boot_keyboard {
                    return true;
                }
            }
            index += 1;
        }
        false
    }
}

static USB_DRIVER_STACK: crate::kernel::sync::SpinLock<UsbDriverStackState> =
    crate::kernel::sync::SpinLock::new(UsbDriverStackState::new());

struct XhciHostModule;
struct UsbCoreModule;
struct UsbHidModule;
struct UsbKeyboardModule;

const XHCI_DEPS: &[crate::kernel::boot_phase::BootPhase] = &[];
const USB_CORE_DEPS: &[crate::kernel::boot_phase::BootPhase] =
    &[crate::kernel::boot_phase::BootPhase::Xhci];
const USB_HID_DEPS: &[crate::kernel::boot_phase::BootPhase] =
    &[crate::kernel::boot_phase::BootPhase::UsbCore];
const USB_KBD_DEPS: &[crate::kernel::boot_phase::BootPhase] =
    &[crate::kernel::boot_phase::BootPhase::UsbHid];

static XHCI_HOST_MODULE_INSTANCE: XhciHostModule = XhciHostModule;
static USB_CORE_MODULE_INSTANCE: UsbCoreModule = UsbCoreModule;
static USB_HID_MODULE_INSTANCE: UsbHidModule = UsbHidModule;
static USB_KEYBOARD_MODULE_INSTANCE: UsbKeyboardModule = UsbKeyboardModule;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsbStackBootStatus {
    pub xhci: DriverStatus,
    pub core: DriverStatus,
    pub hid: DriverStatus,
    pub keyboard: XhciKeyboardStatus,
}

impl DriverModule for XhciHostModule {
    fn descriptor(&self) -> DriverModuleDescriptor {
        DriverModuleDescriptor {
            name: "xhci-host0",
            category: DriverCategory::HostController,
            dependencies: XHCI_DEPS,
            phase: crate::kernel::boot_phase::BootPhase::Xhci,
            required: false,
        }
    }

    fn init(&self) -> Result<(), DriverError> {
        let Some(function) = find_xhci_controller() else {
            crate::kprintln!("[xhci] skipped: no xHCI controller");
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(XHCI_MODULE, DriverStatus::Skipped);
            return Err(DriverError::NoController);
        };

        crate::kprintln!(
            "[xhci] pci device found: {}:{}.{}",
            function.bus,
            function.device,
            function.function
        );
        enable_pci_command(function);
        let Some(bar0) = pci_mmio_bar_base(function, 0x10) else {
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(XHCI_MODULE, DriverStatus::Failed);
            return Err(DriverError::InvalidMmio("MMIO BAR discovery failed"));
        };
        let mmio = current_hhdm_offset()
            .map(|offset| (offset + bar0) as usize)
            .unwrap_or(bar0 as usize);
        crate::kprintln!("[xhci] mmio base: {:#x}", bar0);

        let controller = unsafe { bring_up_xhci(mmio as *mut u8, function, bar0 as usize) }
            .map_err(|error| match error {
                UsbKbdError::InvalidMmio(stage) => DriverError::InvalidMmio(stage),
                UsbKbdError::Timeout(stage) => DriverError::Timeout(stage),
            })?;

        {
            let mut stack = USB_DRIVER_STACK.lock();
            stack.xhci = Some(controller);
            stack
                .registry
                .set_status(XHCI_MODULE, DriverStatus::Initialized);
        }
        crate::kprintln!(
            "[xhci] ports={} slots={} context_size={}",
            controller.max_ports,
            controller.max_slots,
            controller.context_size
        );
        Ok(())
    }

    fn start(&self) -> Result<(), DriverError> {
        if USB_DRIVER_STACK.lock().xhci.is_none() {
            return Err(DriverError::NoController);
        }
        USB_DRIVER_STACK
            .lock()
            .registry
            .set_status(XHCI_MODULE, DriverStatus::Online);
        crate::kprintln!("[xhci] controller online");
        Ok(())
    }

    fn stop(&self) -> Result<(), DriverError> {
        USB_DRIVER_STACK
            .lock()
            .registry
            .set_status(XHCI_MODULE, DriverStatus::Skipped);
        Ok(())
    }

    fn poll(&self) {}

    fn status(&self) -> DriverStatus {
        USB_DRIVER_STACK.lock().registry.status(XHCI_MODULE)
    }
}

impl DriverModule for UsbCoreModule {
    fn descriptor(&self) -> DriverModuleDescriptor {
        DriverModuleDescriptor {
            name: "usb-core0",
            category: DriverCategory::Bus,
            dependencies: USB_CORE_DEPS,
            phase: crate::kernel::boot_phase::BootPhase::UsbCore,
            required: false,
        }
    }

    fn init(&self) -> Result<(), DriverError> {
        if XHCI_HOST_MODULE_INSTANCE.status() == DriverStatus::Skipped {
            crate::kprintln!("[usb] skipped: dependency xhci-host0 skipped");
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_CORE_MODULE, DriverStatus::Skipped);
            return Err(DriverError::DependencySkipped("xhci-host0 skipped"));
        }
        if XHCI_HOST_MODULE_INSTANCE.status() == DriverStatus::Failed {
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_CORE_MODULE, DriverStatus::Failed);
            return Err(DriverError::DependencyFailed("xhci-host0 failed"));
        }
        let controller = match USB_DRIVER_STACK.lock().xhci {
            Some(controller) => controller,
            None => {
                USB_DRIVER_STACK
                    .lock()
                    .registry
                    .set_status(USB_CORE_MODULE, DriverStatus::Skipped);
                return Err(DriverError::DependencySkipped("xhci-host0 unavailable"));
            }
        };

        crate::kprintln!("[usb] port scan: {} ports", controller.max_ports);
        let mut found = 0usize;
        let mut port = 0u8;
        while port < controller.max_ports && found < MAX_USB_DEVICES {
            match unsafe { scan_and_reset_port(controller, port) } {
                Ok(true) => {
                    crate::kprintln!("[usb] device found on port {}", port + 1);
                    let record = UsbDeviceRecord {
                        id: (found + 1) as u8,
                        port,
                        endpoint_count: 1,
                        is_hid_boot_keyboard_candidate: true,
                    };
                    if USB_DRIVER_STACK.lock().add_device(record) {
                        found += 1;
                    }
                }
                Ok(false) => {}
                Err(error) => {
                    USB_DRIVER_STACK
                        .lock()
                        .registry
                        .set_status(USB_CORE_MODULE, DriverStatus::Failed);
                    return Err(match error {
                        UsbKbdError::InvalidMmio(stage) => DriverError::InvalidMmio(stage),
                        UsbKbdError::Timeout(stage) => DriverError::Timeout(stage),
                    });
                }
            }
            port += 1;
        }

        USB_DRIVER_STACK
            .lock()
            .registry
            .set_status(USB_CORE_MODULE, DriverStatus::Initialized);
        Ok(())
    }

    fn start(&self) -> Result<(), DriverError> {
        if XHCI_HOST_MODULE_INSTANCE.status() != DriverStatus::Online {
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_CORE_MODULE, DriverStatus::Skipped);
            return Err(DriverError::DependencySkipped("xhci-host0 not online"));
        }
        USB_DRIVER_STACK
            .lock()
            .registry
            .set_status(USB_CORE_MODULE, DriverStatus::Online);
        Ok(())
    }

    fn stop(&self) -> Result<(), DriverError> {
        USB_DRIVER_STACK
            .lock()
            .registry
            .set_status(USB_CORE_MODULE, DriverStatus::Skipped);
        Ok(())
    }

    fn poll(&self) {}

    fn status(&self) -> DriverStatus {
        USB_DRIVER_STACK.lock().registry.status(USB_CORE_MODULE)
    }
}

impl DriverModule for UsbHidModule {
    fn descriptor(&self) -> DriverModuleDescriptor {
        DriverModuleDescriptor {
            name: "usb-hid0",
            category: DriverCategory::UsbClass,
            dependencies: USB_HID_DEPS,
            phase: crate::kernel::boot_phase::BootPhase::UsbHid,
            required: false,
        }
    }

    fn init(&self) -> Result<(), DriverError> {
        if USB_CORE_MODULE_INSTANCE.status() != DriverStatus::Online
            && USB_CORE_MODULE_INSTANCE.status() != DriverStatus::Initialized
        {
            crate::kprintln!("[usb] HID skipped: dependency usb-core0 not active");
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_HID_MODULE, DriverStatus::Skipped);
            return Err(DriverError::DependencySkipped("usb-core0 skipped"));
        }

        let devices = USB_DRIVER_STACK.lock().devices;
        let mut index = 0usize;
        let mut claimed = 0usize;
        while index < devices.len() && claimed < MAX_HID_DEVICES {
            if let Some(device) = devices[index] {
                if device.is_hid_boot_keyboard_candidate {
                    let hid = HidDeviceRecord {
                        device_id: device.id,
                        interface_number: 0,
                        interrupt_in_endpoint: 1,
                        max_packet_size: 8,
                        boot_keyboard: true,
                    };
                    if USB_DRIVER_STACK.lock().add_hid(hid) {
                        crate::kprintln!("[usb] hid boot keyboard found");
                        claimed += 1;
                    }
                }
            }
            index += 1;
        }

        if claimed == 0 {
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_HID_MODULE, DriverStatus::Skipped);
            return Err(DriverError::NoHid);
        }

        USB_DRIVER_STACK
            .lock()
            .registry
            .set_status(USB_HID_MODULE, DriverStatus::Initialized);
        Ok(())
    }

    fn start(&self) -> Result<(), DriverError> {
        if USB_DRIVER_STACK.lock().hid_count() == 0 {
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_HID_MODULE, DriverStatus::Skipped);
            return Err(DriverError::NoHid);
        }
        USB_DRIVER_STACK
            .lock()
            .registry
            .set_status(USB_HID_MODULE, DriverStatus::Online);
        Ok(())
    }

    fn stop(&self) -> Result<(), DriverError> {
        USB_DRIVER_STACK
            .lock()
            .registry
            .set_status(USB_HID_MODULE, DriverStatus::Skipped);
        Ok(())
    }

    fn poll(&self) {}

    fn status(&self) -> DriverStatus {
        USB_DRIVER_STACK.lock().registry.status(USB_HID_MODULE)
    }
}

impl DriverModule for UsbKeyboardModule {
    fn descriptor(&self) -> DriverModuleDescriptor {
        DriverModuleDescriptor {
            name: "usb-kbd0",
            category: DriverCategory::Input,
            dependencies: USB_KBD_DEPS,
            phase: crate::kernel::boot_phase::BootPhase::UsbKeyboard,
            required: false,
        }
    }

    fn init(&self) -> Result<(), DriverError> {
        if USB_HID_MODULE_INSTANCE.status() != DriverStatus::Online
            && USB_HID_MODULE_INSTANCE.status() != DriverStatus::Initialized
        {
            crate::kprintln!("[usbkbd] skipped: dependency usb-hid0 not active");
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_KBD_MODULE, DriverStatus::Skipped);
            return Err(DriverError::DependencySkipped("usb-hid0 skipped"));
        }
        if !USB_DRIVER_STACK.lock().has_boot_keyboard() {
            crate::kprintln!("[usbkbd] skipped: no HID boot keyboard");
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_KBD_MODULE, DriverStatus::Skipped);
            return Err(DriverError::NoKeyboard);
        }
        crate::kprintln!("[usbkbd] endpoint configured");
        USB_DRIVER_STACK
            .lock()
            .registry
            .set_status(USB_KBD_MODULE, DriverStatus::Initialized);
        Ok(())
    }

    fn start(&self) -> Result<(), DriverError> {
        if !USB_DRIVER_STACK.lock().has_boot_keyboard() {
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_KBD_MODULE, DriverStatus::Skipped);
            return Err(DriverError::NoKeyboard);
        }
        mark_source_online(InputRawSource::UsbHid);
        {
            let mut stack = USB_DRIVER_STACK.lock();
            stack.keyboard_online = true;
            stack
                .registry
                .set_status(USB_KBD_MODULE, DriverStatus::Online);
        }
        crate::kprintln!("[usbkbd] online");
        Ok(())
    }

    fn stop(&self) -> Result<(), DriverError> {
        USB_DRIVER_STACK
            .lock()
            .registry
            .set_status(USB_KBD_MODULE, DriverStatus::Skipped);
        Ok(())
    }

    fn poll(&self) {}

    fn status(&self) -> DriverStatus {
        USB_DRIVER_STACK.lock().registry.status(USB_KBD_MODULE)
    }
}

pub fn initialize_usb_driver_stack(hhdm_offset: Option<u64>) -> UsbStackBootStatus {
    set_current_hhdm_offset(hhdm_offset);
    {
        let mut stack = USB_DRIVER_STACK.lock();
        stack.clear_runtime();
        stack
            .registry
            .register(XHCI_MODULE, XHCI_HOST_MODULE_INSTANCE.descriptor());
        stack
            .registry
            .register(USB_CORE_MODULE, USB_CORE_MODULE_INSTANCE.descriptor());
        stack
            .registry
            .register(USB_HID_MODULE, USB_HID_MODULE_INSTANCE.descriptor());
        stack
            .registry
            .register(USB_KBD_MODULE, USB_KEYBOARD_MODULE_INSTANCE.descriptor());
    }

    run_dependency_gated_module(&XHCI_HOST_MODULE_INSTANCE, XHCI_MODULE);
    run_dependency_gated_module(&USB_CORE_MODULE_INSTANCE, USB_CORE_MODULE);
    run_dependency_gated_module(&USB_HID_MODULE_INSTANCE, USB_HID_MODULE);
    run_dependency_gated_module(&USB_KEYBOARD_MODULE_INSTANCE, USB_KBD_MODULE);

    let stack = USB_DRIVER_STACK.lock();
    let keyboard = match stack.registry.status(USB_KBD_MODULE) {
        DriverStatus::Online => XhciKeyboardStatus::Online,
        DriverStatus::Skipped => {
            if stack.registry.status(XHCI_MODULE) == DriverStatus::Skipped {
                XhciKeyboardStatus::SkippedNoController
            } else {
                XhciKeyboardStatus::SkippedNoKeyboard
            }
        }
        DriverStatus::Failed => XhciKeyboardStatus::Failed("USB keyboard module failed"),
        _ => XhciKeyboardStatus::SkippedNoKeyboard,
    };
    UsbStackBootStatus {
        xhci: stack.registry.status(XHCI_MODULE),
        core: stack.registry.status(USB_CORE_MODULE),
        hid: stack.registry.status(USB_HID_MODULE),
        keyboard,
    }
}

fn run_dependency_gated_module(module: &dyn DriverModule, slot: usize) -> ModuleInitStatus {
    use crate::kernel::boot_phase::{
        boot_phase_failed, boot_phase_ok, boot_phase_online, boot_phase_skipped, boot_phase_start,
        PhaseState,
    };

    let descriptor = module.descriptor();
    let mut dep_index = 0usize;
    while dep_index < descriptor.dependencies.len() {
        let dependency = descriptor.dependencies[dep_index];
        let dependency_state = crate::kernel::boot_phase::boot_phase_state(dependency);
        match dependency_state {
            PhaseState::Ok | PhaseState::Online | PhaseState::Enabled => {}
            PhaseState::Failed => {
                let reason = "required dependency failed";
                let mut stack = USB_DRIVER_STACK.lock();
                stack.registry.set_status(slot, DriverStatus::Skipped);
                drop(stack);
                boot_phase_skipped(descriptor.phase, reason);
                crate::kprintln!(
                    "[usbdrv] {} skipped: dependency {} failed",
                    descriptor.name,
                    dependency.name()
                );
                return ModuleInitStatus::Skipped(reason);
            }
            PhaseState::Skipped
            | PhaseState::Stub
            | PhaseState::Unregistered
            | PhaseState::Registered
            | PhaseState::Pending
            | PhaseState::Started
            | PhaseState::Detected => {
                let reason = "required dependency unavailable";
                let mut stack = USB_DRIVER_STACK.lock();
                stack.registry.set_status(slot, DriverStatus::Skipped);
                drop(stack);
                boot_phase_skipped(descriptor.phase, reason);
                crate::kprintln!(
                    "[usbdrv] {} skipped: dependency {} not online",
                    descriptor.name,
                    dependency.name()
                );
                return ModuleInitStatus::Skipped(reason);
            }
        }
        dep_index += 1;
    }

    boot_phase_start(descriptor.phase);
    let status = match module.init().and_then(|_| module.start()) {
        Ok(()) => match module.status() {
            DriverStatus::Online => ModuleInitStatus::Online,
            DriverStatus::Initialized => ModuleInitStatus::Ok,
            DriverStatus::Skipped => ModuleInitStatus::Skipped("module skipped"),
            DriverStatus::Failed => ModuleInitStatus::Failed("driver module failed"),
            DriverStatus::Registered => ModuleInitStatus::Skipped("driver module did not start"),
        },
        Err(error) => {
            let skipped = matches!(
                error,
                DriverError::DependencySkipped(_)
                    | DriverError::NoController
                    | DriverError::NoDevice
                    | DriverError::NoHid
                    | DriverError::NoKeyboard
            );
            let mut stack = USB_DRIVER_STACK.lock();
            stack.registry.set_status(
                slot,
                if skipped {
                    DriverStatus::Skipped
                } else {
                    DriverStatus::Failed
                },
            );
            drop(stack);
            crate::kprintln!("[usbdrv] {}: {}", descriptor.name, error.message());
            if skipped {
                ModuleInitStatus::Skipped(error.message())
            } else {
                ModuleInitStatus::Failed(error.message())
            }
        }
    };

    match status {
        ModuleInitStatus::Online => boot_phase_online(descriptor.phase),
        ModuleInitStatus::Ok => boot_phase_ok(descriptor.phase),
        ModuleInitStatus::Skipped(message) => boot_phase_skipped(descriptor.phase, message),
        ModuleInitStatus::Failed(message) => boot_phase_failed(descriptor.phase, message),
    }
    status
}

static CURRENT_HHDM_OFFSET: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(u64::MAX);

fn set_current_hhdm_offset(offset: Option<u64>) {
    CURRENT_HHDM_OFFSET.store(
        offset.unwrap_or(u64::MAX),
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn current_hhdm_offset() -> Option<u64> {
    let raw = CURRENT_HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);
    if raw == u64::MAX {
        None
    } else {
        Some(raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XhciKeyboardStatus {
    Online,
    SkippedNoController,
    SkippedNoKeyboard,
    Failed(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UsbKbdError {
    InvalidMmio(&'static str),
    Timeout(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct XhciRegisters {
    op: *mut u8,
    max_ports: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PciFunction {
    bus: u8,
    device: u8,
    function: u8,
}

pub struct UsbHidKeyboardDriver;

impl UsbHidKeyboardDriver {
    pub const fn new() -> Self {
        Self
    }

    pub fn initialize(&self, hhdm_offset: Option<u64>) -> XhciKeyboardStatus {
        self.initialize_stack(hhdm_offset).keyboard
    }

    pub fn initialize_stack(&self, hhdm_offset: Option<u64>) -> UsbStackBootStatus {
        initialize_usb_driver_stack(hhdm_offset)
    }

    pub fn ingest_boot_report(
        &self,
        previous: HidBootKeyboardReport,
        current: HidBootKeyboardReport,
    ) {
        for event in diff_hid_boot_reports(previous, current)
            .into_iter()
            .flatten()
        {
            publish_keyboard_event(event);
            if event.keycode == KeyCode::Escape && event.state == KeyState::Pressed {
                crate::kprintln!("usb-hid-keyboard0: ESC raw={:#x}", event.raw_code);
            }
        }
    }
}

impl DeviceDriver for UsbHidKeyboardDriver {
    fn kind(&self) -> DeviceKind {
        DeviceKind::InputController
    }

    fn name(&self) -> &'static str {
        "usb-hid-keyboard0"
    }

    fn security(&self) -> DeviceSecurity {
        DeviceSecurity::new(SecurityClass::Internal, false)
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, DeviceError> {
        if buffer.len() < core::mem::size_of::<crate::kernel::device::MirageInputEvent>() {
            return Err(DeviceError::BufferTooSmall);
        }
        Ok(copy_mirage_events(buffer))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HidBootKeyboardReport {
    pub modifiers: u8,
    pub reserved: u8,
    pub keys: [u8; 6],
}

pub fn diff_hid_boot_reports(
    previous: HidBootKeyboardReport,
    current: HidBootKeyboardReport,
) -> [Option<KeyboardEvent>; 12] {
    let mut out = [None; 12];
    let mut index = 0usize;
    let modifiers = hid_modifiers(current.modifiers);

    let mut slot = 0usize;
    while slot < 6 {
        let key = previous.keys[slot];
        if key != 0 && !contains_key(current.keys, key) {
            out[index] = hid_usage_to_event(key, KeyState::Released, modifiers);
            index += 1;
        }
        slot += 1;
    }

    slot = 0;
    while slot < 6 {
        let key = current.keys[slot];
        if key != 0 && !contains_key(previous.keys, key) {
            out[index] = hid_usage_to_event(key, KeyState::Pressed, modifiers);
            index += 1;
        }
        slot += 1;
    }
    out
}

fn contains_key(keys: [u8; 6], needle: u8) -> bool {
    let mut index = 0usize;
    while index < keys.len() {
        if keys[index] == needle {
            return true;
        }
        index += 1;
    }
    false
}

pub fn hid_modifiers(bits: u8) -> KeyModifiers {
    KeyModifiers {
        left_shift: bits & (1 << 1) != 0,
        right_shift: bits & (1 << 5) != 0,
        ctrl: bits & ((1 << 0) | (1 << 4)) != 0,
        alt: bits & ((1 << 2) | (1 << 6)) != 0,
        caps_lock: false,
    }
}

pub fn hid_usage_to_event(
    usage: u8,
    state: KeyState,
    modifiers: KeyModifiers,
) -> Option<KeyboardEvent> {
    let keycode = match usage {
        0x04..=0x1d => KeyCode::Char(0),
        0x1e..=0x27 => KeyCode::Char(0),
        0x28 => KeyCode::Enter,
        0x29 => KeyCode::Escape,
        0x2a => KeyCode::Backspace,
        0x2b => KeyCode::Tab,
        0x3a..=0x45 => KeyCode::F(usage - 0x39),
        0x4f => KeyCode::ArrowRight,
        0x50 => KeyCode::ArrowLeft,
        0x51 => KeyCode::ArrowDown,
        0x52 => KeyCode::ArrowUp,
        0xe0 => KeyCode::LeftCtrl,
        0xe1 => KeyCode::LeftShift,
        0xe2 => KeyCode::LeftAlt,
        0xe4 => KeyCode::RightCtrl,
        0xe5 => KeyCode::RightShift,
        0xe6 => KeyCode::RightAlt,
        _ => KeyCode::Raw(usage as u16),
    };
    let ascii = if state == KeyState::Pressed {
        hid_usage_ascii(usage, modifiers)
    } else {
        None
    };
    Some(KeyboardEvent::new(
        keycode,
        state,
        modifiers,
        ascii,
        InputRawSource::UsbHid,
        usage as u16,
    ))
}

pub fn hid_usage_ascii(usage: u8, modifiers: KeyModifiers) -> Option<u8> {
    let shifted = modifiers.shift();
    Some(match usage {
        0x04..=0x1d => {
            let base = b'a' + (usage - 0x04);
            if shifted {
                base - 32
            } else {
                base
            }
        }
        0x1e => {
            if shifted {
                b'!'
            } else {
                b'1'
            }
        }
        0x1f => {
            if shifted {
                b'@'
            } else {
                b'2'
            }
        }
        0x20 => {
            if shifted {
                b'#'
            } else {
                b'3'
            }
        }
        0x21 => {
            if shifted {
                b'$'
            } else {
                b'4'
            }
        }
        0x22 => {
            if shifted {
                b'%'
            } else {
                b'5'
            }
        }
        0x23 => {
            if shifted {
                b'^'
            } else {
                b'6'
            }
        }
        0x24 => {
            if shifted {
                b'&'
            } else {
                b'7'
            }
        }
        0x25 => {
            if shifted {
                b'*'
            } else {
                b'8'
            }
        }
        0x26 => {
            if shifted {
                b'('
            } else {
                b'9'
            }
        }
        0x27 => {
            if shifted {
                b')'
            } else {
                b'0'
            }
        }
        0x28 => b'\n',
        0x2a => 8,
        0x2b => b'\t',
        0x2c => b' ',
        0x2d => {
            if shifted {
                b'_'
            } else {
                b'-'
            }
        }
        0x2e => {
            if shifted {
                b'+'
            } else {
                b'='
            }
        }
        0x2f => {
            if shifted {
                b'{'
            } else {
                b'['
            }
        }
        0x30 => {
            if shifted {
                b'}'
            } else {
                b']'
            }
        }
        0x31 => {
            if shifted {
                b'|'
            } else {
                b'\\'
            }
        }
        0x33 => {
            if shifted {
                b':'
            } else {
                b';'
            }
        }
        0x34 => {
            if shifted {
                b'"'
            } else {
                b'\''
            }
        }
        0x36 => {
            if shifted {
                b'<'
            } else {
                b','
            }
        }
        0x37 => {
            if shifted {
                b'>'
            } else {
                b'.'
            }
        }
        0x38 => {
            if shifted {
                b'?'
            } else {
                b'/'
            }
        }
        _ => return None,
    })
}

fn find_xhci_controller() -> Option<PciFunction> {
    // Trust only bus 0 until bridge bus discovery exists. Follow PCI rules:
    // read function 0 first and scan functions 1..7 only for multifunction devices.
    let bus = 0u8;
    let mut device = 0u8;
    while device <= 31 {
        let function0 = PciFunction {
            bus,
            device,
            function: 0,
        };
        let id0 = pci_read_u32(function0, 0x00);
        if (id0 & 0xffff) as u16 != 0xffff {
            if is_xhci_pci_function(function0) {
                return Some(function0);
            }

            let header_type = ((pci_read_u32(function0, 0x0c) >> 16) & 0xff) as u8;
            if (header_type & 0x80) != 0 {
                let mut function = 1u8;
                while function <= 7 {
                    let candidate = PciFunction {
                        bus,
                        device,
                        function,
                    };
                    let id = pci_read_u32(candidate, 0x00);
                    if (id & 0xffff) as u16 != 0xffff && is_xhci_pci_function(candidate) {
                        return Some(candidate);
                    }
                    function += 1;
                }
            }
        }
        device += 1;
    }
    None
}

fn is_xhci_pci_function(function: PciFunction) -> bool {
    let class_reg = pci_read_u32(function, 0x08);
    let class = (class_reg >> 24) as u8;
    let subclass = (class_reg >> 16) as u8;
    let prog_if = (class_reg >> 8) as u8;
    class == PCI_CLASS_SERIAL_BUS && subclass == PCI_SUBCLASS_USB && prog_if == PCI_PROGIF_XHCI
}

fn pci_mmio_bar_base(function: PciFunction, offset: u8) -> Option<u64> {
    let raw = pci_read_u32(function, offset);
    if raw == 0 || raw == 0xffff_ffff || (raw & 0x1) != 0 {
        return None;
    }

    let memory_type = (raw >> 1) & 0x3;
    let base = match memory_type {
        0x0 => (raw & !0x0f) as u64,
        0x2 => {
            let high = pci_read_u32(function, offset + 4);
            ((high as u64) << 32) | ((raw & !0x0f) as u64)
        }
        _ => return None,
    };

    if base == 0 {
        None
    } else {
        Some(base)
    }
}

fn pci_read_u32(function: PciFunction, offset: u8) -> u32 {
    let address = 0x8000_0000u32
        | ((function.bus as u32) << 16)
        | ((function.device as u32) << 11)
        | ((function.function as u32) << 8)
        | ((offset as u32) & 0xfc);
    unsafe {
        crate::arch::x86_64::io::outl(PCI_CONFIG_ADDRESS, address);
        crate::arch::x86_64::io::inl(PCI_CONFIG_DATA)
    }
}

fn pci_write_u32(function: PciFunction, offset: u8, value: u32) {
    let address = 0x8000_0000u32
        | ((function.bus as u32) << 16)
        | ((function.device as u32) << 11)
        | ((function.function as u32) << 8)
        | ((offset as u32) & 0xfc);
    unsafe {
        crate::arch::x86_64::io::outl(PCI_CONFIG_ADDRESS, address);
        crate::arch::x86_64::io::outl(PCI_CONFIG_DATA, value);
    }
}

fn enable_pci_command(function: PciFunction) {
    let value = pci_read_u32(function, 0x04) | 0x0006;
    pci_write_u32(function, 0x04, value);
}

unsafe fn mmio_read32(base: *mut u8, offset: usize) -> u32 {
    core::ptr::read_volatile(base.add(offset) as *const u32)
}

unsafe fn mmio_write32(base: *mut u8, offset: usize, value: u32) {
    core::ptr::write_volatile(base.add(offset) as *mut u32, value)
}

unsafe fn bring_up_xhci(
    base: *mut u8,
    function: PciFunction,
    mmio_base: usize,
) -> Result<XhciController, UsbKbdError> {
    if base.is_null() {
        return Err(UsbKbdError::InvalidMmio("invalid MMIO base"));
    }
    let cap_length = core::ptr::read_volatile(base as *const u8) as usize;
    if cap_length < 0x20 || cap_length > 0x100 {
        return Err(UsbKbdError::InvalidMmio("invalid xHCI capability length"));
    }
    let op = base.add(cap_length);
    let hcsparams1 = mmio_read32(base, 0x04);
    let hccparams1 = mmio_read32(base, 0x10);
    let dboff = (mmio_read32(base, 0x14) & !0x3) as usize;
    let rtsoff = (mmio_read32(base, 0x18) & !0x1f) as usize;
    if dboff == 0 || rtsoff == 0 {
        return Err(UsbKbdError::InvalidMmio(
            "invalid xHCI runtime/doorbell offsets",
        ));
    }

    let mut cmd = mmio_read32(op, 0x00);
    cmd &= !USBCMD_RUN;
    mmio_write32(op, 0x00, cmd);
    wait_status(op, USBSTS_HCH, true, "timeout waiting for controller halt")?;

    mmio_write32(op, 0x00, cmd | USBCMD_RESET);
    wait_command_clear(op, USBCMD_RESET, "timeout waiting for controller reset")?;

    let max_slots = (hcsparams1 & 0xff).min(32) as u8;
    let max_ports = ((hcsparams1 >> 24) & 0xff).min(32) as u8;
    if max_ports == 0 {
        return Err(UsbKbdError::InvalidMmio("xHCI reports zero root ports"));
    }
    let context_size = if hccparams1 & (1 << 2) != 0 { 64 } else { 32 };
    configure_static_xhci_rings(op)?;
    mmio_write32(op, 0x38, max_slots as u32);

    cmd = mmio_read32(op, 0x00) | USBCMD_RUN;
    mmio_write32(op, 0x00, cmd);
    wait_status(op, USBSTS_HCH, false, "timeout waiting for controller run")?;
    Ok(XhciController {
        function,
        mmio_base,
        cap: base as usize,
        op: op as usize,
        runtime: base as usize + rtsoff,
        doorbells: base as usize + dboff,
        max_ports,
        max_slots,
        context_size,
    })
}

#[repr(C, align(64))]
struct XhciAlignedU64<const N: usize>([u64; N]);

static mut XHCI_DCBAA: XhciAlignedU64<256> = XhciAlignedU64([0; 256]);
static mut XHCI_COMMAND_RING: XhciAlignedU64<64> = XhciAlignedU64([0; 64]);
static mut XHCI_EVENT_RING: XhciAlignedU64<64> = XhciAlignedU64([0; 64]);
static mut XHCI_ERST: XhciAlignedU64<2> = XhciAlignedU64([0; 2]);

unsafe fn configure_static_xhci_rings(op: *mut u8) -> Result<(), UsbKbdError> {
    let dcbaa = core::ptr::addr_of_mut!(XHCI_DCBAA.0) as u64;
    let command_ring = (core::ptr::addr_of_mut!(XHCI_COMMAND_RING.0) as u64) | 1;
    let event_ring = core::ptr::addr_of_mut!(XHCI_EVENT_RING.0) as u64;
    let erst = core::ptr::addr_of_mut!(XHCI_ERST.0) as u64;

    XHCI_ERST.0[0] = event_ring;
    XHCI_ERST.0[1] = 64;

    mmio_write32(op, 0x30, dcbaa as u32);
    mmio_write32(op, 0x34, (dcbaa >> 32) as u32);
    mmio_write32(op, 0x18, command_ring as u32);
    mmio_write32(op, 0x1c, (command_ring >> 32) as u32);

    // Interrupter 0 ERST registers are under RTSOFF + 0x20, but Mirage still
    // lacks a generic xHCI interrupt owner. Keep command/event backing prepared
    // without submitting commands from this boot path.
    let _ = erst;
    Ok(())
}

unsafe fn scan_and_reset_port(controller: XhciController, port: u8) -> Result<bool, UsbKbdError> {
    let registers = XhciRegisters {
        op: controller.op as *mut u8,
        max_ports: controller.max_ports,
    };
    let portsc = mmio_read32(registers.op, portsc_offset(port));
    if portsc & PORTSC_CCS == 0 {
        return Ok(false);
    }
    reset_port(registers, port)?;
    Ok(true)
}

unsafe fn reset_port(registers: XhciRegisters, port: u8) -> Result<(), UsbKbdError> {
    let offset = portsc_offset(port);
    let mut portsc = mmio_read32(registers.op, offset);
    if portsc & PORTSC_CCS == 0 {
        return Err(UsbKbdError::InvalidMmio("port reset target disconnected"));
    }

    portsc |= PORTSC_PP | PORTSC_PR;
    mmio_write32(registers.op, offset, portsc);
    wait_port_bit(
        registers.op,
        offset,
        PORTSC_PR,
        false,
        "timeout waiting for port reset",
    )?;
    wait_port_bit(
        registers.op,
        offset,
        PORTSC_PED,
        true,
        "timeout waiting for port enable",
    )
}

const fn portsc_offset(port: u8) -> usize {
    PORT_REGISTER_BASE + (port as usize * PORT_REGISTER_STRIDE)
}

unsafe fn wait_command_clear(
    op: *mut u8,
    bit: u32,
    stage: &'static str,
) -> Result<(), UsbKbdError> {
    let mut wait = 0usize;
    while wait < WAIT_LIMIT {
        if mmio_read32(op, 0x00) & bit == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
        wait += 1;
    }
    Err(UsbKbdError::Timeout(stage))
}

unsafe fn wait_status(
    op: *mut u8,
    bit: u32,
    set: bool,
    stage: &'static str,
) -> Result<(), UsbKbdError> {
    let mut wait = 0usize;
    while wait < WAIT_LIMIT {
        let present = mmio_read32(op, 0x04) & bit != 0;
        if present == set {
            return Ok(());
        }
        core::hint::spin_loop();
        wait += 1;
    }
    Err(UsbKbdError::Timeout(stage))
}

unsafe fn wait_port_bit(
    op: *mut u8,
    offset: usize,
    bit: u32,
    set: bool,
    stage: &'static str,
) -> Result<(), UsbKbdError> {
    let mut wait = 0usize;
    while wait < WAIT_LIMIT {
        let present = mmio_read32(op, offset) & bit != 0;
        if present == set {
            return Ok(());
        }
        core::hint::spin_loop();
        wait += 1;
    }
    Err(UsbKbdError::Timeout(stage))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsbDeviceDescriptor {
    pub vendor_id: u16,
    pub product_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub max_packet_size_ep0: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsbInterfaceDescriptor {
    pub number: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsbEndpointDescriptor {
    pub address: u8,
    pub attributes: u8,
    pub max_packet_size: u16,
    pub interval: u8,
}

impl UsbEndpointDescriptor {
    pub const fn is_interrupt_in(self) -> bool {
        self.address & 0x80 != 0 && self.attributes & 0x03 == 0x03
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsbConfigurationScan {
    pub total_length: u16,
    pub hid_boot_keyboard: Option<UsbInterfaceDescriptor>,
    pub interrupt_in: Option<UsbEndpointDescriptor>,
}

pub fn parse_device_descriptor(bytes: &[u8]) -> Result<UsbDeviceDescriptor, DriverError> {
    if bytes.len() < 18 || bytes[0] < 18 || bytes[1] != 1 {
        return Err(DriverError::DescriptorMalformed);
    }
    Ok(UsbDeviceDescriptor {
        vendor_id: u16::from_le_bytes([bytes[8], bytes[9]]),
        product_id: u16::from_le_bytes([bytes[10], bytes[11]]),
        class: bytes[4],
        subclass: bytes[5],
        protocol: bytes[6],
        max_packet_size_ep0: bytes[7],
    })
}

pub fn scan_configuration_descriptor(bytes: &[u8]) -> Result<UsbConfigurationScan, DriverError> {
    if bytes.len() < 9 || bytes[0] < 9 || bytes[1] != 2 {
        return Err(DriverError::DescriptorMalformed);
    }
    let total_length = u16::from_le_bytes([bytes[2], bytes[3]]);
    if total_length as usize > bytes.len() || total_length < 9 {
        return Err(DriverError::DescriptorMalformed);
    }

    let mut scan = UsbConfigurationScan {
        total_length,
        hid_boot_keyboard: None,
        interrupt_in: None,
    };
    let mut offset = 0usize;
    while offset < total_length as usize {
        let remaining = total_length as usize - offset;
        if remaining < 2 {
            return Err(DriverError::DescriptorMalformed);
        }
        let length = bytes[offset] as usize;
        let descriptor_type = bytes[offset + 1];
        if length < 2 || length > remaining {
            return Err(DriverError::DescriptorMalformed);
        }
        match descriptor_type {
            4 if length >= 9 => {
                let interface = UsbInterfaceDescriptor {
                    number: bytes[offset + 2],
                    class: bytes[offset + 5],
                    subclass: bytes[offset + 6],
                    protocol: bytes[offset + 7],
                };
                if interface.class == 0x03
                    && interface.subclass == 0x01
                    && interface.protocol == 0x01
                {
                    scan.hid_boot_keyboard = Some(interface);
                }
            }
            5 if length >= 7 => {
                let endpoint = UsbEndpointDescriptor {
                    address: bytes[offset + 2],
                    attributes: bytes[offset + 3],
                    max_packet_size: u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]),
                    interval: bytes[offset + 6],
                };
                if endpoint.is_interrupt_in() && endpoint.max_packet_size >= 8 {
                    scan.interrupt_in = Some(endpoint);
                }
            }
            _ => {}
        }
        offset += length;
    }
    Ok(scan)
}

pub static USB_HID_KEYBOARD_DRIVER: UsbHidKeyboardDriver = UsbHidKeyboardDriver::new();

pub fn mark_usb_keyboard_online_for_enumeration() {
    mark_source_online(InputRawSource::UsbHid);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hid_report_diff_generates_press_and_release() {
        let prev = HidBootKeyboardReport {
            modifiers: 0,
            reserved: 0,
            keys: [0x04, 0, 0, 0, 0, 0],
        };
        let curr = HidBootKeyboardReport {
            modifiers: 0,
            reserved: 0,
            keys: [0x29, 0, 0, 0, 0, 0],
        };
        let events = diff_hid_boot_reports(prev, curr);
        assert_eq!(events[0].unwrap().state, KeyState::Released);
        assert_eq!(events[1].unwrap().keycode, KeyCode::Escape);
        assert_eq!(events[1].unwrap().state, KeyState::Pressed);
    }

    #[test]
    fn hid_modifier_translation_supports_shift() {
        let mods = hid_modifiers(1 << 1);
        assert!(mods.left_shift);
        assert_eq!(hid_usage_ascii(0x04, mods), Some(b'A'));
    }

    #[test]
    fn descriptor_parser_finds_boot_keyboard_interrupt_endpoint() {
        let config = [
            9, 2, 25, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 1, 0x03, 0x01, 0x01, 0, 7, 5, 0x81, 0x03,
            8, 0, 10,
        ];
        let scan = scan_configuration_descriptor(&config).unwrap();
        assert!(scan.hid_boot_keyboard.is_some());
        assert!(scan.interrupt_in.unwrap().is_interrupt_in());
    }

    #[test]
    fn descriptor_parser_rejects_non_advancing_descriptor() {
        let malformed = [9, 2, 11, 0, 1, 1, 0, 0x80, 50, 0, 4];
        assert_eq!(
            scan_configuration_descriptor(&malformed),
            Err(DriverError::DescriptorMalformed)
        );
    }
}
