//! Modular hardware-backed USB driver stack for early Mirage input.
//!
//! This file keeps the current x86_64 boot path no-heap and bounded while
//! splitting USB into kernel-registered modules: `xhci-host0`, `usb-core0`,
//! `usb-hid0`, and `usb-kbd0`.  The boundaries mirror the future supervised
//! driver-service ownership model and avoid the old fragile inline HID init path.

use crate::kernel::device::{DeviceDriver, DeviceError, DeviceKind};
use crate::kernel::input::{
    copy_mirage_events, mark_source_online, publish_keyboard_event, InputRawSource, KeyCode,
    KeyModifiers, KeyState, KeyboardEvent,
};
use crate::subkernel::{DeviceSecurity, SecurityClass};
use mirage_platform::{
    PlatformDevice, PlatformLocation, PlatformRegistry, MAX_PLATFORM_DEVICE_EVENTS,
};

const PCI_CONFIG_ADDRESS: u16 = 0xcf8;
const PCI_CONFIG_DATA: u16 = 0xcfc;
pub const PCI_CLASS_SERIAL_BUS: u8 = 0x0c;
pub const PCI_SUBCLASS_USB: u8 = 0x03;
pub const PCI_PROGIF_XHCI: u8 = 0x30;

const USBSTS_HCH: u32 = 1 << 0;
const USBSTS_EINT: u32 = 1 << 3;
const USBCMD_RUN: u32 = 1 << 0;
const USBCMD_RESET: u32 = 1 << 1;
const USBCMD_INTE: u32 = 1 << 2;
const PORTSC_CCS: u32 = 1 << 0;
const PORTSC_PED: u32 = 1 << 1;
const PORTSC_PR: u32 = 1 << 4;
const PORTSC_PP: u32 = 1 << 9;
const PORT_REGISTER_STRIDE: usize = 0x10;
const PORT_REGISTER_BASE: usize = 0x400;
const WAIT_LIMIT: usize = 1_000_000;
const EVENT_POLL_LIMIT: usize = 250_000;
const MAX_USB_DEVICES: usize = 8;
const MAX_HID_DEVICES: usize = 4;
const MAX_STORAGE_DEVICES: usize = 4;
const XHCI_PAGE_SIZE: usize = crate::kernel::memory::PAGE_SIZE;
const XHCI_MAX_SCRATCHPADS: usize = 1024;

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
    Started,
    Initialized,
    Online,
    Pending,
    Skipped,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleInitStatus {
    Online,
    Ok,
    Pending(&'static str),
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
    DmaAllocationFailed,
    DependencySkipped(&'static str),
    DependencyFailed(&'static str),
    NoController,
    NoDevice,
    NoConnectedPorts,
    PortResetTimeout,
    AddressDeviceFailed,
    DescriptorReadFailed,
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
            Self::DmaAllocationFailed => "DMA allocation failed",
            Self::DependencySkipped(name) => name,
            Self::DependencyFailed(name) => name,
            Self::NoController => "no xHCI controller",
            Self::NoDevice => "no USB devices",
            Self::NoConnectedPorts => "no connected USB ports",
            Self::PortResetTimeout => "port reset timeout",
            Self::AddressDeviceFailed => "address-device failure",
            Self::DescriptorReadFailed => "descriptor read failure",
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
    fn preflight_skip(&self) -> Option<&'static str> {
        None
    }

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
    command_index: usize,
    command_cycle: bool,
    event_cycle: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UsbDeviceRecord {
    id: u8,
    slot_id: u8,
    port: u8,
    endpoint_count: u8,
    port_speed: u8,
    hid_interface_number: u8,
    interrupt_in_endpoint: u8,
    interrupt_in_max_packet_size: u16,
    interrupt_in_interval: u8,
    configuration_value: u8,
    is_hid_boot_keyboard_candidate: bool,
    storage_interface_number: u8,
    bulk_in_endpoint: u8,
    bulk_in_max_packet_size: u16,
    bulk_out_endpoint: u8,
    bulk_out_max_packet_size: u16,
    is_mass_storage_bot_candidate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HidDeviceRecord {
    device_id: u8,
    slot_id: u8,
    interface_number: u8,
    interrupt_in_endpoint: u8,
    max_packet_size: u16,
    interval: u8,
    configuration_value: u8,
    polling_live: bool,
    previous_report: HidBootKeyboardReport,
    boot_keyboard: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UsbStorageDeviceRecord {
    device_id: u8,
    slot_id: u8,
    interface_number: u8,
    bulk_in_endpoint: u8,
    bulk_in_max_packet_size: u16,
    bulk_out_endpoint: u8,
    bulk_out_max_packet_size: u16,
    configuration_value: u8,
    bot_ready: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UsbDriverStackState {
    registry: DriverRegistry,
    xhci: Option<XhciController>,
    devices: [Option<UsbDeviceRecord>; MAX_USB_DEVICES],
    hids: [Option<HidDeviceRecord>; MAX_HID_DEVICES],
    storage_devices: [Option<UsbStorageDeviceRecord>; MAX_STORAGE_DEVICES],
    keyboard_online: bool,
}

impl UsbDriverStackState {
    pub const fn new() -> Self {
        Self {
            registry: DriverRegistry::new(),
            xhci: None,
            devices: [None; MAX_USB_DEVICES],
            hids: [None; MAX_HID_DEVICES],
            storage_devices: [None; MAX_STORAGE_DEVICES],
            keyboard_online: false,
        }
    }

    fn clear_runtime(&mut self) {
        self.xhci = None;
        self.devices = [None; MAX_USB_DEVICES];
        self.hids = [None; MAX_HID_DEVICES];
        self.storage_devices = [None; MAX_STORAGE_DEVICES];
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

    fn add_storage(&mut self, record: UsbStorageDeviceRecord) -> bool {
        let mut index = 0usize;
        while index < self.storage_devices.len() {
            if self.storage_devices[index].is_none() {
                self.storage_devices[index] = Some(record);
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

    fn has_live_boot_keyboard_polling(&self) -> bool {
        let mut index = 0usize;
        while index < self.hids.len() {
            if let Some(record) = self.hids[index] {
                if record.boot_keyboard && record.polling_live {
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

    fn preflight_skip(&self) -> Option<&'static str> {
        if selected_xhci_function().is_none() {
            Some("no xHCI controller")
        } else {
            None
        }
    }

    fn init(&self) -> Result<(), DriverError> {
        let Some(function) = selected_xhci_function() else {
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
        let Some(bar0) = selected_xhci_mmio_bar().or_else(|| pci_mmio_bar_base(function, 0x10))
        else {
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
                UsbKbdError::DmaAllocationFailed => {
                    crate::kprintln!("AMD XHCI RINGS [FAILED: DMA allocation failed]");
                    DriverError::DmaAllocationFailed
                }
                UsbKbdError::InvalidMmio(stage) => DriverError::InvalidMmio(stage),
                UsbKbdError::Timeout(stage) => DriverError::Timeout(stage),
            })?;

        {
            let mut stack = USB_DRIVER_STACK.lock();
            stack.xhci = Some(controller);
            stack
                .registry
                .set_status(XHCI_MODULE, DriverStatus::Started);
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
        let mut stack = USB_DRIVER_STACK.lock();
        let Some(mut controller) = stack.xhci else {
            return Err(DriverError::NoController);
        };
        drop(stack);
        unsafe { submit_noop_command_and_wait(&mut controller) }.map_err(|error| match error {
            UsbKbdError::DmaAllocationFailed => DriverError::DmaAllocationFailed,
            UsbKbdError::InvalidMmio(stage) => DriverError::InvalidMmio(stage),
            UsbKbdError::Timeout(stage) => DriverError::Timeout(stage),
        })?;
        let mut stack = USB_DRIVER_STACK.lock();
        stack.xhci = Some(controller);
        stack.registry.set_status(XHCI_MODULE, DriverStatus::Online);
        crate::kprintln!("[xhci] command/event rings ok; irq mode: polling");
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
        if XHCI_HOST_MODULE_INSTANCE.status() != DriverStatus::Online {
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_CORE_MODULE, DriverStatus::Skipped);
            return Err(DriverError::DependencySkipped("xhci-host0 not online"));
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

        unsafe { validate_root_hub_register_access(controller) }.map_err(|error| match error {
            UsbKbdError::DmaAllocationFailed => DriverError::DmaAllocationFailed,
            UsbKbdError::InvalidMmio(stage) => DriverError::InvalidMmio(stage),
            UsbKbdError::Timeout(stage) => DriverError::Timeout(stage),
        })?;
        crate::kprintln!(
            "[usb] root hub registers validated; ports={}",
            controller.max_ports
        );
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
        if self.status() != DriverStatus::Initialized {
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_CORE_MODULE, DriverStatus::Skipped);
            return Err(DriverError::DependencySkipped(
                "usb-core0 root hub registers not validated",
            ));
        }

        let mut stack = USB_DRIVER_STACK.lock();
        let Some(mut controller) = stack.xhci else {
            stack
                .registry
                .set_status(USB_CORE_MODULE, DriverStatus::Skipped);
            return Err(DriverError::DependencySkipped("xhci-host0 unavailable"));
        };
        drop(stack);

        crate::kprintln!("[usb] port scan: {} ports", controller.max_ports);
        let mut connected = 0usize;
        let mut reset = 0usize;
        let mut enumerated = 0usize;
        let mut saw_address_failure = false;
        let mut saw_descriptor_failure = false;
        let mut port = 0u8;
        while port < controller.max_ports && enumerated < MAX_USB_DEVICES {
            match unsafe { port_connected(controller, port) } {
                Ok(true) => {
                    connected += 1;
                    crate::kprintln!("[usb] connected port {}", port + 1);
                    if let Err(error) = unsafe { reset_connected_port(controller, port) } {
                        USB_DRIVER_STACK
                            .lock()
                            .registry
                            .set_status(USB_CORE_MODULE, DriverStatus::Failed);
                        crate::kprintln!(
                            "[usb] port {} reset failed: {}",
                            port + 1,
                            error.message()
                        );
                        return Err(match error {
                            UsbKbdError::DmaAllocationFailed => DriverError::DmaAllocationFailed,
                            UsbKbdError::InvalidMmio(stage) => DriverError::InvalidMmio(stage),
                            UsbKbdError::Timeout(_) => DriverError::PortResetTimeout,
                        });
                    }
                    reset += 1;
                    crate::kprintln!("[usb] port {} reset complete", port + 1);
                    let device_id = (enumerated + 1) as u8;
                    match unsafe { enumerate_usb_device(&mut controller, device_id, port) } {
                        Ok(record) => {
                            crate::kprintln!(
                                "[usb] device {} descriptor enumeration succeeded",
                                device_id
                            );
                            if USB_DRIVER_STACK.lock().add_device(record) {
                                enumerated += 1;
                            }
                        }
                        Err(UsbEnumerationError::AddressDeviceFailed) => {
                            saw_address_failure = true;
                            crate::kprintln!(
                                "USB DEVICE {} [FAILED: address-device failure]",
                                device_id
                            );
                        }
                        Err(UsbEnumerationError::DescriptorReadFailed) => {
                            saw_descriptor_failure = true;
                            crate::kprintln!(
                                "USB DEVICE {} [FAILED: descriptor read failure]",
                                device_id
                            );
                        }
                        Err(UsbEnumerationError::EndpointSetupFailed) => {
                            saw_descriptor_failure = true;
                            crate::kprintln!(
                                "USB DEVICE {} [FAILED: endpoint setup failed before descriptor completion]",
                                device_id
                            );
                        }
                    }
                }
                Ok(false) => {}
                Err(error) => {
                    USB_DRIVER_STACK
                        .lock()
                        .registry
                        .set_status(USB_CORE_MODULE, DriverStatus::Failed);
                    return Err(match error {
                        UsbKbdError::DmaAllocationFailed => DriverError::DmaAllocationFailed,
                        UsbKbdError::InvalidMmio(stage) => DriverError::InvalidMmio(stage),
                        UsbKbdError::Timeout(stage) => DriverError::Timeout(stage),
                    });
                }
            }
            port += 1;
        }
        crate::kprintln!(
            "[usb] port scan completed: connected={} reset={} enumerated={}",
            connected,
            reset,
            enumerated
        );

        let mut stack = USB_DRIVER_STACK.lock();
        stack.xhci = Some(controller);
        if enumerated > 0 {
            stack
                .registry
                .set_status(USB_CORE_MODULE, DriverStatus::Online);
            crate::kprintln!("[usb] online: successful descriptor enumeration");
            Ok(())
        } else if connected == 0 {
            stack
                .registry
                .set_status(USB_CORE_MODULE, DriverStatus::Skipped);
            crate::kprintln!("[usb] skipped: no connected ports");
            Err(DriverError::NoConnectedPorts)
        } else if saw_address_failure {
            stack
                .registry
                .set_status(USB_CORE_MODULE, DriverStatus::Failed);
            Err(DriverError::AddressDeviceFailed)
        } else if saw_descriptor_failure {
            stack
                .registry
                .set_status(USB_CORE_MODULE, DriverStatus::Failed);
            Err(DriverError::DescriptorReadFailed)
        } else {
            stack
                .registry
                .set_status(USB_CORE_MODULE, DriverStatus::Failed);
            Err(DriverError::NoDevice)
        }
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
                        interface_number: device.hid_interface_number,
                        slot_id: device.slot_id,
                        interrupt_in_endpoint: device.interrupt_in_endpoint,
                        max_packet_size: device.interrupt_in_max_packet_size,
                        interval: device.interrupt_in_interval,
                        configuration_value: device.configuration_value,
                        polling_live: false,
                        previous_report: HidBootKeyboardReport::default(),
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
            detect_usb_mass_storage_after_descriptor_enumeration();
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_HID_MODULE, DriverStatus::Skipped);
            return Err(DriverError::NoHid);
        }
        detect_usb_mass_storage_after_descriptor_enumeration();

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

fn detect_usb_mass_storage_after_descriptor_enumeration() {
    let devices = USB_DRIVER_STACK.lock().devices;
    let mut index = 0usize;
    while index < devices.len() {
        if let Some(device) = devices[index] {
            if device.is_mass_storage_bot_candidate {
                let record = UsbStorageDeviceRecord {
                    device_id: device.id,
                    slot_id: device.slot_id,
                    interface_number: device.storage_interface_number,
                    bulk_in_endpoint: device.bulk_in_endpoint,
                    bulk_in_max_packet_size: device.bulk_in_max_packet_size,
                    bulk_out_endpoint: device.bulk_out_endpoint,
                    bulk_out_max_packet_size: device.bulk_out_max_packet_size,
                    configuration_value: device.configuration_value,
                    bot_ready: false,
                };
                if USB_DRIVER_STACK.lock().add_storage(record) {
                    crate::kprintln!(
                        "USB STORAGE [DETECTED] device={} slot={} iface={} cfg={} bot_ready={} bulk-in=0x{:02x}/{} bulk-out=0x{:02x}/{}",
                        record.device_id,
                        record.slot_id,
                        record.interface_number,
                        record.configuration_value,
                        record.bot_ready,
                        record.bulk_in_endpoint,
                        record.bulk_in_max_packet_size,
                        record.bulk_out_endpoint,
                        record.bulk_out_max_packet_size
                    );
                    crate::kprintln!("USB STORAGE [PENDING: BOT not implemented]");
                }
            }
        }
        index += 1;
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
        let mut stack = USB_DRIVER_STACK.lock();
        let Some(mut controller) = stack.xhci else {
            stack
                .registry
                .set_status(USB_KBD_MODULE, DriverStatus::Failed);
            return Err(DriverError::NoController);
        };
        let mut hids = stack.hids;
        drop(stack);

        let mut configured = false;
        let mut index = 0usize;
        while index < hids.len() {
            if let Some(mut hid) = hids[index] {
                unsafe { configure_hid_boot_keyboard_endpoint(&mut controller, &mut hid) }
                    .map_err(|_| DriverError::EndpointSetupFailed)?;
                hid.polling_live = true;
                hids[index] = Some(hid);
                configured = true;
                crate::kprintln!(
                    "[usbkbd] interrupt IN ep={} max_packet={} interval={} configured",
                    u32::from(hid.interrupt_in_endpoint & 0x0f),
                    hid.max_packet_size,
                    hid.interval
                );
            }
            index += 1;
        }

        let mut stack = USB_DRIVER_STACK.lock();
        stack.xhci = Some(controller);
        stack.hids = hids;
        if !configured {
            stack
                .registry
                .set_status(USB_KBD_MODULE, DriverStatus::Skipped);
            return Err(DriverError::NoKeyboard);
        }
        stack
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
        poll_configured_hid_keyboards();
        if !USB_DRIVER_STACK.lock().has_live_boot_keyboard_polling() {
            USB_DRIVER_STACK
                .lock()
                .registry
                .set_status(USB_KBD_MODULE, DriverStatus::Failed);
            return Err(DriverError::EndpointSetupFailed);
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

    fn poll(&self) {
        poll_configured_hid_keyboards();
    }

    fn status(&self) -> DriverStatus {
        USB_DRIVER_STACK.lock().registry.status(USB_KBD_MODULE)
    }
}

pub fn initialize_usb_driver_stack_with_platform(
    hhdm_offset: Option<u64>,
    platform: &PlatformRegistry<MAX_PLATFORM_DEVICE_EVENTS>,
) -> UsbStackBootStatus {
    select_xhci_from_platform(platform);
    initialize_usb_driver_stack(hhdm_offset)
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
        DriverStatus::Pending | DriverStatus::Started | DriverStatus::Initialized => {
            XhciKeyboardStatus::SkippedNoKeyboard
        }
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
        boot_phase_failed, boot_phase_ok, boot_phase_online, boot_phase_pending,
        boot_phase_skipped, boot_phase_start, PhaseState,
    };

    let descriptor = module.descriptor();
    let mut dep_index = 0usize;
    while dep_index < descriptor.dependencies.len() {
        let dependency = descriptor.dependencies[dep_index];
        let dependency_state = crate::kernel::boot_phase::boot_phase_state(dependency);
        match dependency_state {
            PhaseState::Ok | PhaseState::Online | PhaseState::Enabled | PhaseState::Running => {}
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
            | PhaseState::Detected
            | PhaseState::Found => {
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

    if let Some(reason) = module.preflight_skip() {
        let mut stack = USB_DRIVER_STACK.lock();
        stack.registry.set_status(slot, DriverStatus::Skipped);
        drop(stack);
        boot_phase_skipped(descriptor.phase, reason);
        crate::kprintln!("[usbdrv] {} skipped: {}", descriptor.name, reason);
        return ModuleInitStatus::Skipped(reason);
    }

    boot_phase_start(descriptor.phase);
    let status = match module.init().and_then(|_| module.start()) {
        Ok(()) => match module.status() {
            DriverStatus::Online => ModuleInitStatus::Online,
            DriverStatus::Started => {
                ModuleInitStatus::Pending("controller started; event path pending")
            }
            DriverStatus::Initialized => ModuleInitStatus::Ok,
            DriverStatus::Pending => ModuleInitStatus::Pending("driver pending"),
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
                    | DriverError::NoConnectedPorts
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
        ModuleInitStatus::Pending(message) => boot_phase_pending(descriptor.phase, message),
        ModuleInitStatus::Skipped(message) => boot_phase_skipped(descriptor.phase, message),
        ModuleInitStatus::Failed(message) => boot_phase_failed(descriptor.phase, message),
    }
    status
}

static CURRENT_HHDM_OFFSET: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(u64::MAX);
static SELECTED_XHCI_FUNCTION: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);
static SELECTED_XHCI_MMIO_BAR: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

fn select_xhci_from_platform(platform: &PlatformRegistry<MAX_PLATFORM_DEVICE_EVENTS>) {
    if let Some(device) = platform.platform_find_xhci_controller() {
        if let Some(function) = pci_function_from_platform_device(device) {
            SELECTED_XHCI_FUNCTION.store(
                encode_pci_function(function),
                core::sync::atomic::Ordering::Relaxed,
            );
            let mmio = device.mmio_bar(0).map(|bar| bar.base).unwrap_or(0);
            SELECTED_XHCI_MMIO_BAR.store(mmio, core::sync::atomic::Ordering::Relaxed);
            return;
        }
    }
    SELECTED_XHCI_FUNCTION.store(u32::MAX, core::sync::atomic::Ordering::Relaxed);
    SELECTED_XHCI_MMIO_BAR.store(0, core::sync::atomic::Ordering::Relaxed);
}

fn pci_function_from_platform_device(device: PlatformDevice) -> Option<PciFunction> {
    match device.location {
        PlatformLocation::Pci {
            bus,
            device,
            function,
        } => Some(PciFunction {
            bus,
            device,
            function,
        }),
        _ => None,
    }
}

const fn encode_pci_function(function: PciFunction) -> u32 {
    ((function.bus as u32) << 16) | ((function.device as u32) << 8) | function.function as u32
}

const fn decode_pci_function(raw: u32) -> PciFunction {
    PciFunction {
        bus: ((raw >> 16) & 0xff) as u8,
        device: ((raw >> 8) & 0xff) as u8,
        function: (raw & 0xff) as u8,
    }
}

fn selected_xhci_function() -> Option<PciFunction> {
    let raw = SELECTED_XHCI_FUNCTION.load(core::sync::atomic::Ordering::Relaxed);
    if raw == u32::MAX {
        None
    } else {
        Some(decode_pci_function(raw))
    }
}

fn selected_xhci_mmio_bar() -> Option<u64> {
    let raw = SELECTED_XHCI_MMIO_BAR.load(core::sync::atomic::Ordering::Relaxed);
    if raw == 0 {
        None
    } else {
        Some(raw)
    }
}

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
    DmaAllocationFailed,
    InvalidMmio(&'static str),
    Timeout(&'static str),
}

impl UsbKbdError {
    const fn message(self) -> &'static str {
        match self {
            Self::DmaAllocationFailed => "xHCI DMA allocation failed",
            Self::InvalidMmio(stage) | Self::Timeout(stage) => stage,
        }
    }
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

    pub fn initialize_stack_with_platform(
        &self,
        hhdm_offset: Option<u64>,
        platform: &PlatformRegistry<MAX_PLATFORM_DEVICE_EVENTS>,
    ) -> UsbStackBootStatus {
        initialize_usb_driver_stack_with_platform(hhdm_offset, platform)
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
        meta: bits & ((1 << 3) | (1 << 7)) != 0,
        caps_lock: false,
        num_lock: false,
        scroll_lock: false,
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

pub const fn is_xhci_class(class: u8, subclass: u8, prog_if: u8) -> bool {
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
    let hcsparams2 = mmio_read32(base, 0x08);
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
    let max_scratchpads = decode_max_scratchpad_buffers(hcsparams2);
    configure_static_xhci_rings(op, base.add(rtsoff), max_scratchpads)?;
    mmio_write32(op, 0x38, max_slots as u32);

    cmd = mmio_read32(op, 0x00) | USBCMD_RUN | USBCMD_INTE;
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
        command_index: 0,
        command_cycle: true,
        event_cycle: true,
    })
}

#[derive(Clone, Copy)]
#[repr(C, align(64))]
struct XhciAlignedU64<const N: usize>([u64; N]);

#[derive(Clone, Copy)]
#[repr(C, align(4096))]
struct XhciAlignedPageU64<const N: usize>([u64; N]);

#[derive(Clone, Copy)]
#[repr(C, align(4096))]
struct XhciPage([u8; XHCI_PAGE_SIZE]);

static mut XHCI_DCBAA: XhciAlignedU64<256> = XhciAlignedU64([0; 256]);
static mut XHCI_SCRATCHPAD_POINTERS: XhciAlignedPageU64<XHCI_MAX_SCRATCHPADS> =
    XhciAlignedPageU64([0; XHCI_MAX_SCRATCHPADS]);
static mut XHCI_SCRATCHPAD_BUFFERS: [XhciPage; XHCI_MAX_SCRATCHPADS] =
    [const { XhciPage([0; XHCI_PAGE_SIZE]) }; XHCI_MAX_SCRATCHPADS];
static mut XHCI_COMMAND_RING: XhciAlignedU64<64> = XhciAlignedU64([0; 64]);
static mut XHCI_EVENT_RING: XhciAlignedU64<64> = XhciAlignedU64([0; 64]);
static mut XHCI_ERST: XhciAlignedU64<2> = XhciAlignedU64([0; 2]);
static mut XHCI_INPUT_CONTEXTS: [XhciAlignedU64<64>; MAX_USB_DEVICES] =
    [const { XhciAlignedU64([0; 64]) }; MAX_USB_DEVICES];
static mut XHCI_DEVICE_CONTEXTS: [XhciAlignedU64<64>; MAX_USB_DEVICES] =
    [const { XhciAlignedU64([0; 64]) }; MAX_USB_DEVICES];
static mut XHCI_EP0_TRANSFER_RINGS: [XhciAlignedU64<64>; MAX_USB_DEVICES] =
    [const { XhciAlignedU64([0; 64]) }; MAX_USB_DEVICES];
static mut XHCI_EP0_DEQUEUE_INDEX: [usize; MAX_USB_DEVICES] = [0; MAX_USB_DEVICES];
static mut XHCI_EP0_CYCLE: [bool; MAX_USB_DEVICES] = [true; MAX_USB_DEVICES];
static mut XHCI_INTERRUPT_IN_RINGS: [XhciAlignedU64<64>; MAX_USB_DEVICES] =
    [const { XhciAlignedU64([0; 64]) }; MAX_USB_DEVICES];
static mut XHCI_INTERRUPT_IN_DEQUEUE_INDEX: [usize; MAX_USB_DEVICES] = [0; MAX_USB_DEVICES];
static mut XHCI_INTERRUPT_IN_CYCLE: [bool; MAX_USB_DEVICES] = [true; MAX_USB_DEVICES];
static mut USB_DESCRIPTOR_BUFFER: [XhciAlignedU64<64>; MAX_USB_DEVICES] =
    [const { XhciAlignedU64([0; 64]) }; MAX_USB_DEVICES];
static mut USB_INTERRUPT_REPORT_BUFFER: [XhciAlignedU64<1>; MAX_USB_DEVICES] =
    [const { XhciAlignedU64([0; 1]) }; MAX_USB_DEVICES];

/// Bounded early-boot xHCI DMA reservation ledger.
///
/// This is intentionally not a general heap allocator.  The early x86_64 xHCI
/// path still uses fixed, aligned BSS objects so boot can run before the kernel
/// heap and before a supervised `usbd` DMA/IOMMU allocator exist.  The arena
/// ledger accounts every xHCI DMA object against a small fixed byte budget,
/// verifies the requested alignment, and translates the virtual address before
/// any xHCI register receives it.  The migration point for supervised `usbd`
/// is this interface: replace the fixed-object backend with capability-owned
/// DMA allocations while preserving the `dma_for_static` contract.
#[derive(Clone, Copy)]
struct XhciEarlyDmaArena {
    cursor: usize,
}

const XHCI_EARLY_DMA_ARENA_BYTES: usize = core::mem::size_of::<XhciAlignedU64<256>>()
    + core::mem::size_of::<XhciAlignedPageU64<XHCI_MAX_SCRATCHPADS>>()
    + core::mem::size_of::<[XhciPage; XHCI_MAX_SCRATCHPADS]>()
    + core::mem::size_of::<XhciAlignedU64<64>>() * (3 + (MAX_USB_DEVICES * 5))
    + core::mem::size_of::<XhciAlignedU64<1>>() * MAX_USB_DEVICES;

impl XhciEarlyDmaArena {
    const fn new() -> Self {
        Self { cursor: 0 }
    }

    fn reset(&mut self) {
        self.cursor = 0;
    }

    fn dma_for_static(
        &mut self,
        virtual_address: u64,
        bytes: usize,
        alignment: usize,
    ) -> Result<u64, UsbKbdError> {
        if bytes == 0
            || !alignment.is_power_of_two()
            || virtual_address as usize & (alignment - 1) != 0
            || self.cursor.checked_add(bytes).is_none()
            || self.cursor + bytes > XHCI_EARLY_DMA_ARENA_BYTES
        {
            return Err(UsbKbdError::DmaAllocationFailed);
        }

        let dma = try_dma_address(virtual_address).ok_or(UsbKbdError::DmaAllocationFailed)?;
        let pages = (bytes + XHCI_PAGE_SIZE - 1) / XHCI_PAGE_SIZE;
        let mut page = 1usize;
        while page < pages {
            let page_virt = virtual_address + (page * XHCI_PAGE_SIZE) as u64;
            let page_dma = try_dma_address(page_virt).ok_or(UsbKbdError::DmaAllocationFailed)?;
            if page_dma != dma + (page * XHCI_PAGE_SIZE) as u64 {
                return Err(UsbKbdError::DmaAllocationFailed);
            }
            page += 1;
        }

        self.cursor += bytes;
        Ok(dma)
    }
}

static mut XHCI_EARLY_DMA_ARENA: XhciEarlyDmaArena = XhciEarlyDmaArena::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UsbEnumerationError {
    AddressDeviceFailed,
    DescriptorReadFailed,
    EndpointSetupFailed,
}

const fn decode_max_scratchpad_buffers(hcsparams2: u32) -> usize {
    let high = ((hcsparams2 >> 21) & 0x1f) as usize;
    let low = ((hcsparams2 >> 27) & 0x1f) as usize;
    (high << 5) | low
}

unsafe fn configure_static_xhci_rings(
    op: *mut u8,
    runtime: *mut u8,
    max_scratchpads: usize,
) -> Result<(), UsbKbdError> {
    XHCI_EARLY_DMA_ARENA.reset();
    XHCI_DCBAA.0.fill(0);
    configure_xhci_scratchpads(max_scratchpads)?;
    XHCI_COMMAND_RING.0.fill(0);
    XHCI_EVENT_RING.0.fill(0);
    XHCI_ERST.0.fill(0);
    let command_ring_virt = core::ptr::addr_of_mut!(XHCI_COMMAND_RING.0) as u64;
    let command_ring_dma = xhci_dma_address(
        command_ring_virt,
        core::mem::size_of::<XhciAlignedU64<64>>(),
        64,
    )?;
    XHCI_COMMAND_RING.0[62] = command_ring_dma;
    XHCI_COMMAND_RING.0[63] = ((6u64) << 10) | (1 << 1) | 1;

    let dcbaa = xhci_dma_address(
        core::ptr::addr_of_mut!(XHCI_DCBAA.0) as u64,
        core::mem::size_of::<XhciAlignedU64<256>>(),
        64,
    )?;
    let command_ring = command_ring_dma | 1;
    let event_ring = xhci_dma_address(
        core::ptr::addr_of_mut!(XHCI_EVENT_RING.0) as u64,
        core::mem::size_of::<XhciAlignedU64<64>>(),
        64,
    )?;
    let erst = xhci_dma_address(
        core::ptr::addr_of_mut!(XHCI_ERST.0) as u64,
        core::mem::size_of::<XhciAlignedU64<2>>(),
        64,
    )?;

    XHCI_ERST.0[0] = event_ring;
    XHCI_ERST.0[1] = 64;

    mmio_write32(op, 0x30, dcbaa as u32);
    mmio_write32(op, 0x34, (dcbaa >> 32) as u32);
    mmio_write32(op, 0x18, command_ring as u32);
    mmio_write32(op, 0x1c, (command_ring >> 32) as u32);

    let interrupter = runtime.add(0x20);
    mmio_write32(interrupter, 0x00, 0x3);
    mmio_write32(interrupter, 0x04, 0);
    mmio_write32(interrupter, 0x08, 1);
    mmio_write32(interrupter, 0x10, erst as u32);
    mmio_write32(interrupter, 0x14, (erst >> 32) as u32);
    mmio_write32(interrupter, 0x18, event_ring as u32);
    mmio_write32(interrupter, 0x1c, ((event_ring >> 32) as u32) | (1 << 3));
    Ok(())
}

/// Configure xHCI scratchpad buffers before the controller is started.
///
/// Mirage's early xHCI path uses the bounded static DMA arena ledger for the
/// scratchpad pointer array and buffers.  The pages are part of the kernel
/// image/BSS reservation, are page-aligned, accounted against the xHCI DMA
/// budget, and translated to DMA-visible physical addresses before they are
/// handed to the controller.
///
/// IOMMU assumption: this early boot driver does not program per-device IOMMU
/// mappings.  It therefore requires either no active IOMMU translation for the
/// xHCI PCI function or an existing identity/DMA mapping that makes these
/// physical addresses reachable by the controller before USBCMD.RUN is set.
/// Once Mirage grows a supervised DMA/IOMMU allocator, this static reservation
/// should move behind that capability-managed allocator.
unsafe fn configure_xhci_scratchpads(max_scratchpads: usize) -> Result<(), UsbKbdError> {
    XHCI_SCRATCHPAD_POINTERS.0.fill(0);
    if max_scratchpads == 0 {
        XHCI_DCBAA.0[0] = 0;
        return Ok(());
    }
    if max_scratchpads > XHCI_MAX_SCRATCHPADS {
        return Err(UsbKbdError::InvalidMmio(
            "xHCI scratchpad count exceeds reserved DMA capacity",
        ));
    }

    let pointer_array_virt = core::ptr::addr_of_mut!(XHCI_SCRATCHPAD_POINTERS.0) as u64;
    if pointer_array_virt as usize & (XHCI_PAGE_SIZE - 1) != 0 {
        return Err(UsbKbdError::InvalidMmio(
            "xHCI scratchpad pointer array is not page aligned",
        ));
    }
    let pointer_array_bytes = max_scratchpads * core::mem::size_of::<u64>();
    let pointer_array_dma =
        xhci_dma_address(pointer_array_virt, pointer_array_bytes, XHCI_PAGE_SIZE)?;

    let mut index = 0usize;
    while index < max_scratchpads {
        XHCI_SCRATCHPAD_BUFFERS[index].0.fill(0);
        let buffer_virt = core::ptr::addr_of_mut!(XHCI_SCRATCHPAD_BUFFERS[index].0) as u64;
        if buffer_virt as usize & (XHCI_PAGE_SIZE - 1) != 0 {
            return Err(UsbKbdError::InvalidMmio(
                "xHCI scratchpad buffer is not page aligned",
            ));
        }
        let buffer_dma = xhci_dma_address(buffer_virt, XHCI_PAGE_SIZE, XHCI_PAGE_SIZE)?;
        XHCI_SCRATCHPAD_POINTERS.0[index] = buffer_dma;
        index += 1;
    }

    XHCI_DCBAA.0[0] = pointer_array_dma;
    Ok(())
}

unsafe fn xhci_dma_address(
    virtual_address: u64,
    bytes: usize,
    alignment: usize,
) -> Result<u64, UsbKbdError> {
    XHCI_EARLY_DMA_ARENA.dma_for_static(virtual_address, bytes, alignment)
}

unsafe fn dma_address(virtual_address: u64) -> Result<u64, UsbKbdError> {
    xhci_dma_address(virtual_address, 8, 8)
}

fn try_dma_address(virtual_address: u64) -> Option<u64> {
    crate::arch::x86_64::paging::translate_kernel_address(virtual_address)
}

unsafe fn submit_noop_command_and_wait(controller: &mut XhciController) -> Result<(), UsbKbdError> {
    submit_command_and_wait(
        controller,
        0,
        0,
        TRB_TYPE_NOOP_COMMAND,
        "xHCI No-Op command",
    )
    .map(|_| ())
}

unsafe fn enumerate_usb_device(
    controller: &mut XhciController,
    device_id: u8,
    port: u8,
) -> Result<UsbDeviceRecord, UsbEnumerationError> {
    let slot_id = enable_slot(controller).map_err(|_| UsbEnumerationError::AddressDeviceFailed)?;
    let port_speed = read_port_speed(*controller, port);
    prepare_device_contexts(controller, device_id, slot_id, port, port_speed)
        .map_err(|_| UsbEnumerationError::EndpointSetupFailed)?;
    address_device(controller, device_id, slot_id)
        .map_err(|_| UsbEnumerationError::AddressDeviceFailed)?;

    let mut device_descriptor = [0u8; 18];
    ep0_get_descriptor(controller, device_id, slot_id, 1, 0, &mut device_descriptor)
        .map_err(|_| UsbEnumerationError::DescriptorReadFailed)?;
    let parsed_device = parse_device_descriptor(&device_descriptor)
        .map_err(|_| UsbEnumerationError::DescriptorReadFailed)?;

    let mut config_header = [0u8; 9];
    ep0_get_descriptor(controller, device_id, slot_id, 2, 0, &mut config_header)
        .map_err(|_| UsbEnumerationError::DescriptorReadFailed)?;
    let total_length = u16::from_le_bytes([config_header[2], config_header[3]]) as usize;
    if total_length < 9 || total_length > 512 {
        return Err(UsbEnumerationError::DescriptorReadFailed);
    }
    let descriptor_words = &mut USB_DESCRIPTOR_BUFFER[(device_id - 1) as usize].0;
    descriptor_words.fill(0);
    let config_bytes =
        core::slice::from_raw_parts_mut(descriptor_words.as_mut_ptr() as *mut u8, 512);
    ep0_get_descriptor(
        controller,
        device_id,
        slot_id,
        2,
        0,
        &mut config_bytes[..total_length],
    )
    .map_err(|_| UsbEnumerationError::DescriptorReadFailed)?;
    let config = scan_configuration_descriptor(&config_bytes[..total_length])
        .map_err(|_| UsbEnumerationError::DescriptorReadFailed)?;

    crate::kprintln!(
        "USB DEVICE {} [OK: {:04x}:{:04x} port={} speed={}]",
        device_id,
        parsed_device.vendor_id,
        parsed_device.product_id,
        port + 1,
        port_speed
    );

    Ok(UsbDeviceRecord {
        id: device_id,
        slot_id,
        port,
        endpoint_count: 1
            + u8::from(config.interrupt_in.is_some())
            + u8::from(config.bulk_in.is_some())
            + u8::from(config.bulk_out.is_some()),
        port_speed,
        hid_interface_number: config
            .hid_boot_keyboard
            .map(|interface| interface.number)
            .unwrap_or(0),
        interrupt_in_endpoint: config
            .interrupt_in
            .map(|endpoint| endpoint.address)
            .unwrap_or(0),
        interrupt_in_max_packet_size: config
            .interrupt_in
            .map(|endpoint| endpoint.max_packet_size)
            .unwrap_or(0),
        interrupt_in_interval: config
            .interrupt_in
            .map(|endpoint| endpoint.interval)
            .unwrap_or(0),
        configuration_value: config.configuration_value,
        is_hid_boot_keyboard_candidate: config.hid_boot_keyboard.is_some()
            && config.interrupt_in.is_some(),
        storage_interface_number: config
            .mass_storage_bot
            .map(|interface| interface.number)
            .unwrap_or(0),
        bulk_in_endpoint: config.bulk_in.map(|endpoint| endpoint.address).unwrap_or(0),
        bulk_in_max_packet_size: config
            .bulk_in
            .map(|endpoint| endpoint.max_packet_size)
            .unwrap_or(0),
        bulk_out_endpoint: config
            .bulk_out
            .map(|endpoint| endpoint.address)
            .unwrap_or(0),
        bulk_out_max_packet_size: config
            .bulk_out
            .map(|endpoint| endpoint.max_packet_size)
            .unwrap_or(0),
        is_mass_storage_bot_candidate: config.mass_storage_bot.is_some()
            && config.bulk_in.is_some()
            && config.bulk_out.is_some(),
    })
}

unsafe fn enable_slot(controller: &mut XhciController) -> Result<u8, UsbKbdError> {
    submit_command_and_wait(
        controller,
        0,
        0,
        TRB_TYPE_ENABLE_SLOT_COMMAND,
        "xHCI Enable Slot command",
    )
    .and_then(|event| {
        if event.slot_id == 0 {
            Err(UsbKbdError::InvalidMmio("xHCI Enable Slot returned slot 0"))
        } else {
            Ok(event.slot_id)
        }
    })
}

unsafe fn address_device(
    controller: &mut XhciController,
    device_id: u8,
    slot_id: u8,
) -> Result<(), UsbKbdError> {
    let input_context = dma_address(core::ptr::addr_of_mut!(
        XHCI_INPUT_CONTEXTS[(device_id - 1) as usize].0
    ) as u64)?;
    let control = ((slot_id as u64) << 24) | ((TRB_TYPE_ADDRESS_DEVICE_COMMAND as u64) << 10) | 1;
    submit_command_and_wait_raw(
        controller,
        input_context,
        0,
        control,
        "xHCI Address Device command",
    )
    .and_then(|_| Ok(()))
}

#[derive(Clone, Copy)]
struct XhciCommandEvent {
    completion_code: u8,
    slot_id: u8,
}

unsafe fn submit_command_and_wait(
    controller: &mut XhciController,
    parameter: u64,
    status: u32,
    trb_type: u8,
    stage: &'static str,
) -> Result<XhciCommandEvent, UsbKbdError> {
    submit_command_and_wait_raw(
        controller,
        parameter,
        status,
        ((trb_type as u64) << 10) | 1,
        stage,
    )
}

unsafe fn submit_command_and_wait_raw(
    controller: &mut XhciController,
    parameter: u64,
    status: u32,
    control: u64,
    stage: &'static str,
) -> Result<XhciCommandEvent, UsbKbdError> {
    XHCI_EVENT_RING.0[0] = 0;
    XHCI_EVENT_RING.0[1] = 0;
    let index = controller.command_index.min(30);
    let cycle_control = (control & !1) | u64::from(controller.command_cycle);
    XHCI_COMMAND_RING.0[index * 2] = parameter;
    XHCI_COMMAND_RING.0[index * 2 + 1] = ((status as u64) & 0xffff_ffff) | (cycle_control << 32);
    mmio_write32(controller.doorbells as *mut u8, 0, 0);
    let event = wait_command_completion(controller, stage)?;
    controller.command_index += 1;
    if controller.command_index >= 31 {
        controller.command_index = 0;
        controller.command_cycle = !controller.command_cycle;
    }
    Ok(event)
}

unsafe fn wait_command_completion(
    controller: &mut XhciController,
    stage: &'static str,
) -> Result<XhciCommandEvent, UsbKbdError> {
    let mut count = 0usize;
    while count < EVENT_POLL_LIMIT {
        let event1 = core::ptr::read_volatile(core::ptr::addr_of!(XHCI_EVENT_RING.0[1]));
        let status = event1 as u32;
        let control = (event1 >> 32) as u32;
        if (control & 1) == controller.event_cycle as u32
            && ((control >> 10) & 0x3f) as u8 == TRB_TYPE_COMMAND_COMPLETION_EVENT
        {
            let completion_code = ((status >> 24) & 0xff) as u8;
            let slot_id = ((control >> 24) & 0xff) as u8;
            acknowledge_event(*controller, 1)?;
            if completion_code == 1 {
                return Ok(XhciCommandEvent {
                    completion_code,
                    slot_id,
                });
            }
            return Err(UsbKbdError::InvalidMmio(stage));
        }
        core::hint::spin_loop();
        count += 1;
    }
    Err(UsbKbdError::Timeout(stage))
}

unsafe fn acknowledge_event(
    controller: XhciController,
    consumed_trbs: usize,
) -> Result<(), UsbKbdError> {
    let erdp =
        dma_address(core::ptr::addr_of!(XHCI_EVENT_RING.0[consumed_trbs * 2]) as u64)? | (1 << 3);
    let interrupter = (controller.runtime as *mut u8).add(0x20);
    mmio_write32(interrupter, 0x18, erdp as u32);
    mmio_write32(interrupter, 0x1c, (erdp >> 32) as u32);
    let sts = mmio_read32(controller.op as *mut u8, 0x04);
    mmio_write32(controller.op as *mut u8, 0x04, sts | USBSTS_EINT);
    Ok(())
}

unsafe fn prepare_device_contexts(
    controller: &XhciController,
    device_id: u8,
    slot_id: u8,
    port: u8,
    port_speed: u8,
) -> Result<(), UsbKbdError> {
    if device_id == 0 || device_id as usize > MAX_USB_DEVICES || slot_id == 0 {
        return Err(UsbKbdError::InvalidMmio("invalid USB device slot"));
    }
    let index = (device_id - 1) as usize;
    XHCI_INPUT_CONTEXTS[index].0.fill(0);
    XHCI_DEVICE_CONTEXTS[index].0.fill(0);
    XHCI_EP0_TRANSFER_RINGS[index].0.fill(0);
    XHCI_EP0_DEQUEUE_INDEX[index] = 0;
    XHCI_EP0_CYCLE[index] = true;
    XHCI_INTERRUPT_IN_RINGS[index].0.fill(0);
    XHCI_INTERRUPT_IN_DEQUEUE_INDEX[index] = 0;
    XHCI_INTERRUPT_IN_CYCLE[index] = true;
    USB_INTERRUPT_REPORT_BUFFER[index].0.fill(0);

    let device_context =
        dma_address(core::ptr::addr_of_mut!(XHCI_DEVICE_CONTEXTS[index].0) as u64)?;
    XHCI_DCBAA.0[slot_id as usize] = device_context;

    write_context_u32(controller, &mut XHCI_INPUT_CONTEXTS[index], 0, 0, 0);
    write_context_u32(controller, &mut XHCI_INPUT_CONTEXTS[index], 0, 1, 0x3);

    let route_string = 0u32;
    let slot_info = route_string | ((port_speed as u32) << 20) | (1 << 27);
    write_context_u32(controller, &mut XHCI_INPUT_CONTEXTS[index], 1, 0, slot_info);
    write_context_u32(
        controller,
        &mut XHCI_INPUT_CONTEXTS[index],
        1,
        1,
        (port as u32 + 1) << 16,
    );

    let ep0_ring_dma =
        dma_address(core::ptr::addr_of_mut!(XHCI_EP0_TRANSFER_RINGS[index].0) as u64)?;
    XHCI_EP0_TRANSFER_RINGS[index].0[62] = ep0_ring_dma;
    XHCI_EP0_TRANSFER_RINGS[index].0[63] = ((6u64) << 42) | (1u64 << 33) | 1;

    let ep0_type_control = 4u32 << 3;
    write_context_u32(
        controller,
        &mut XHCI_INPUT_CONTEXTS[index],
        2,
        1,
        ep0_type_control | (3 << 1) | (8 << 16),
    );
    write_context_u32(
        controller,
        &mut XHCI_INPUT_CONTEXTS[index],
        2,
        2,
        (ep0_ring_dma as u32) | 1,
    );
    write_context_u32(
        controller,
        &mut XHCI_INPUT_CONTEXTS[index],
        2,
        3,
        (ep0_ring_dma >> 32) as u32,
    );
    write_context_u32(controller, &mut XHCI_INPUT_CONTEXTS[index], 2, 4, 8);
    Ok(())
}

fn write_context_u32<const N: usize>(
    controller: &XhciController,
    context: &mut XhciAlignedU64<N>,
    context_index: usize,
    dword: usize,
    value: u32,
) {
    let byte_offset = context_index * controller.context_size as usize + dword * 4;
    let word = byte_offset / 8;
    if word >= N {
        return;
    }
    if byte_offset & 4 == 0 {
        context.0[word] = (context.0[word] & 0xffff_ffff_0000_0000) | value as u64;
    } else {
        context.0[word] = (context.0[word] & 0x0000_0000_ffff_ffff) | ((value as u64) << 32);
    }
}

unsafe fn ep0_get_descriptor(
    controller: &mut XhciController,
    device_id: u8,
    slot_id: u8,
    descriptor_type: u8,
    descriptor_index: u8,
    out: &mut [u8],
) -> Result<(), UsbKbdError> {
    if out.is_empty() || out.len() > 512 {
        return Err(UsbKbdError::InvalidMmio("invalid descriptor buffer"));
    }
    let index = (device_id - 1) as usize;
    let buffer = &mut USB_DESCRIPTOR_BUFFER[index].0;
    buffer.fill(0);
    let buffer_dma = dma_address(buffer.as_mut_ptr() as u64)?;
    let ring = &mut XHCI_EP0_TRANSFER_RINGS[index].0;
    let start = XHCI_EP0_DEQUEUE_INDEX[index].min(28);
    let cycle = u64::from(XHCI_EP0_CYCLE[index]);
    let setup = 0x80u64
        | ((6u64) << 8)
        | (((descriptor_index as u64) | ((descriptor_type as u64) << 8)) << 16)
        | ((out.len() as u64) << 48);
    ring[start * 2] = setup;
    ring[start * 2 + 1] = ((8u64) | ((2u64) << 16)) | (((2u64 << 10) | (3u64 << 16) | cycle) << 32);
    ring[start * 2 + 2] = buffer_dma;
    ring[start * 2 + 3] = (out.len() as u64) | (((3u64 << 10) | (1u64 << 16) | cycle) << 32);
    ring[start * 2 + 4] = 0;
    ring[start * 2 + 5] = ((4u64 << 10) | (1u64 << 16) | cycle) << 32;
    XHCI_EVENT_RING.0[0] = 0;
    XHCI_EVENT_RING.0[1] = 0;
    mmio_write32(
        (controller.doorbells as *mut u8).add(slot_id as usize * 4),
        0,
        1,
    );
    wait_transfer_completion(controller, slot_id, "USB descriptor control transfer")?;
    XHCI_EP0_DEQUEUE_INDEX[index] += 3;
    if XHCI_EP0_DEQUEUE_INDEX[index] >= 29 {
        XHCI_EP0_DEQUEUE_INDEX[index] = 0;
        XHCI_EP0_CYCLE[index] = !XHCI_EP0_CYCLE[index];
    }
    let bytes = core::slice::from_raw_parts(buffer.as_ptr() as *const u8, out.len());
    out.copy_from_slice(bytes);
    Ok(())
}

unsafe fn wait_transfer_completion(
    controller: &mut XhciController,
    slot_id: u8,
    stage: &'static str,
) -> Result<(), UsbKbdError> {
    let mut count = 0usize;
    while count < EVENT_POLL_LIMIT {
        let event1 = core::ptr::read_volatile(core::ptr::addr_of!(XHCI_EVENT_RING.0[1]));
        let status = event1 as u32;
        let control = (event1 >> 32) as u32;
        if (control & 1) == controller.event_cycle as u32
            && ((control >> 10) & 0x3f) as u8 == TRB_TYPE_TRANSFER_EVENT
            && ((control >> 24) & 0xff) as u8 == slot_id
        {
            acknowledge_event(*controller, 1)?;
            if ((status >> 24) & 0xff) as u8 == 1 {
                return Ok(());
            }
            return Err(UsbKbdError::InvalidMmio(stage));
        }
        core::hint::spin_loop();
        count += 1;
    }
    Err(UsbKbdError::Timeout(stage))
}

unsafe fn ep0_control_no_data(
    controller: &mut XhciController,
    device_id: u8,
    slot_id: u8,
    request_type: u8,
    request: u8,
    value: u16,
    index_value: u16,
    stage: &'static str,
) -> Result<(), UsbKbdError> {
    let index = (device_id - 1) as usize;
    let ring = &mut XHCI_EP0_TRANSFER_RINGS[index].0;
    let start = XHCI_EP0_DEQUEUE_INDEX[index].min(28);
    let cycle = u64::from(XHCI_EP0_CYCLE[index]);
    let setup = (request_type as u64)
        | ((request as u64) << 8)
        | ((value as u64) << 16)
        | ((index_value as u64) << 32);
    ring[start * 2] = setup;
    ring[start * 2 + 1] = ((8u64) | ((2u64) << 16)) | (((2u64 << 10) | cycle) << 32);
    ring[start * 2 + 2] = 0;
    ring[start * 2 + 3] = ((4u64 << 10) | (1u64 << 16) | cycle) << 32;
    XHCI_EVENT_RING.0[0] = 0;
    XHCI_EVENT_RING.0[1] = 0;
    mmio_write32(
        (controller.doorbells as *mut u8).add(slot_id as usize * 4),
        0,
        1,
    );
    wait_transfer_completion(controller, slot_id, stage)?;
    XHCI_EP0_DEQUEUE_INDEX[index] += 2;
    if XHCI_EP0_DEQUEUE_INDEX[index] >= 29 {
        XHCI_EP0_DEQUEUE_INDEX[index] = 0;
        XHCI_EP0_CYCLE[index] = !XHCI_EP0_CYCLE[index];
    }
    Ok(())
}

unsafe fn configure_hid_boot_keyboard_endpoint(
    controller: &mut XhciController,
    hid: &mut HidDeviceRecord,
) -> Result<(), UsbKbdError> {
    ep0_control_no_data(
        controller,
        hid.device_id,
        hid.slot_id,
        0x00,
        9,
        hid.configuration_value as u16,
        0,
        "USB Set Configuration",
    )?;
    ep0_control_no_data(
        controller,
        hid.device_id,
        hid.slot_id,
        0x21,
        11,
        0,
        hid.interface_number as u16,
        "HID Set Protocol boot",
    )?;
    ep0_control_no_data(
        controller,
        hid.device_id,
        hid.slot_id,
        0x21,
        10,
        0,
        hid.interface_number as u16,
        "HID Set Idle",
    )?;
    configure_interrupt_in_endpoint_context(controller, hid)?;
    prime_interrupt_in_transfer(controller, hid)?;
    Ok(())
}

unsafe fn configure_interrupt_in_endpoint_context(
    controller: &mut XhciController,
    hid: &HidDeviceRecord,
) -> Result<(), UsbKbdError> {
    let device_index = (hid.device_id - 1) as usize;
    let endpoint_number = (hid.interrupt_in_endpoint & 0x0f) as usize;
    if endpoint_number == 0 || endpoint_number > 15 || hid.max_packet_size < 8 {
        return Err(UsbKbdError::InvalidMmio("invalid HID interrupt endpoint"));
    }
    let endpoint_context_index = endpoint_number * 2 + 1;
    XHCI_INPUT_CONTEXTS[device_index].0.fill(0);
    write_context_u32(
        controller,
        &mut XHCI_INPUT_CONTEXTS[device_index],
        0,
        1,
        1u32 << endpoint_context_index,
    );
    let ring_dma =
        dma_address(core::ptr::addr_of_mut!(XHCI_INTERRUPT_IN_RINGS[device_index].0) as u64)?;
    XHCI_INTERRUPT_IN_RINGS[device_index].0[62] = ring_dma;
    XHCI_INTERRUPT_IN_RINGS[device_index].0[63] = ((6u64) << 42) | (1u64 << 33) | 1;
    let interval = hid.interval.max(1) as u32;
    let ep_type_interrupt_in = 7u32 << 3;
    write_context_u32(
        controller,
        &mut XHCI_INPUT_CONTEXTS[device_index],
        endpoint_context_index,
        0,
        interval << 16,
    );
    write_context_u32(
        controller,
        &mut XHCI_INPUT_CONTEXTS[device_index],
        endpoint_context_index,
        1,
        ep_type_interrupt_in | (3 << 1) | ((hid.max_packet_size as u32) << 16),
    );
    write_context_u32(
        controller,
        &mut XHCI_INPUT_CONTEXTS[device_index],
        endpoint_context_index,
        2,
        (ring_dma as u32) | 1,
    );
    write_context_u32(
        controller,
        &mut XHCI_INPUT_CONTEXTS[device_index],
        endpoint_context_index,
        3,
        (ring_dma >> 32) as u32,
    );
    write_context_u32(
        controller,
        &mut XHCI_INPUT_CONTEXTS[device_index],
        endpoint_context_index,
        4,
        hid.max_packet_size as u32,
    );
    let input_context =
        dma_address(core::ptr::addr_of_mut!(XHCI_INPUT_CONTEXTS[device_index].0) as u64)?;
    let control =
        ((hid.slot_id as u64) << 24) | ((TRB_TYPE_CONFIGURE_ENDPOINT_COMMAND as u64) << 10) | 1;
    submit_command_and_wait_raw(
        controller,
        input_context,
        0,
        control,
        "xHCI Configure Endpoint command",
    )?;
    Ok(())
}

unsafe fn prime_interrupt_in_transfer(
    controller: &mut XhciController,
    hid: &HidDeviceRecord,
) -> Result<(), UsbKbdError> {
    let index = (hid.device_id - 1) as usize;
    let ring = &mut XHCI_INTERRUPT_IN_RINGS[index].0;
    let start = XHCI_INTERRUPT_IN_DEQUEUE_INDEX[index].min(30);
    let cycle = u64::from(XHCI_INTERRUPT_IN_CYCLE[index]);
    let report = &mut USB_INTERRUPT_REPORT_BUFFER[index].0;
    report.fill(0);
    let report_dma = dma_address(report.as_mut_ptr() as u64)?;
    ring[start * 2] = report_dma;
    ring[start * 2 + 1] = (8u64) | (((1u64 << 10) | (1u64 << 5) | cycle) << 32);
    XHCI_EVENT_RING.0[0] = 0;
    XHCI_EVENT_RING.0[1] = 0;
    mmio_write32(
        (controller.doorbells as *mut u8).add(hid.slot_id as usize * 4),
        0,
        u32::from(hid.interrupt_in_endpoint & 0x0f),
    );
    Ok(())
}

fn poll_configured_hid_keyboards() {
    let mut stack = USB_DRIVER_STACK.lock();
    let Some(mut controller) = stack.xhci else {
        return;
    };
    let mut hids = stack.hids;
    drop(stack);
    let mut index = 0usize;
    while index < hids.len() {
        if let Some(mut hid) = hids[index] {
            if hid.polling_live {
                unsafe {
                    let _ = poll_hid_keyboard_once(&mut controller, &mut hid);
                }
                hids[index] = Some(hid);
            }
        }
        index += 1;
    }
    let mut stack = USB_DRIVER_STACK.lock();
    stack.xhci = Some(controller);
    stack.hids = hids;
}

unsafe fn poll_hid_keyboard_once(
    controller: &mut XhciController,
    hid: &mut HidDeviceRecord,
) -> Result<(), UsbKbdError> {
    let mut count = 0usize;
    while count < 128 {
        let event1 = core::ptr::read_volatile(core::ptr::addr_of!(XHCI_EVENT_RING.0[1]));
        let status = event1 as u32;
        let control = (event1 >> 32) as u32;
        if (control & 1) == controller.event_cycle as u32
            && ((control >> 10) & 0x3f) as u8 == TRB_TYPE_TRANSFER_EVENT
            && ((control >> 24) & 0xff) as u8 == hid.slot_id
        {
            acknowledge_event(*controller, 1)?;
            if ((status >> 24) & 0xff) as u8 == 1 {
                let index = (hid.device_id - 1) as usize;
                let bytes = core::slice::from_raw_parts(
                    USB_INTERRUPT_REPORT_BUFFER[index].0.as_ptr() as *const u8,
                    8,
                );
                if let Some(report) = decode_boot_keyboard_report(bytes) {
                    for event in diff_hid_boot_reports(hid.previous_report, report)
                        .into_iter()
                        .flatten()
                    {
                        publish_keyboard_event(event);
                    }
                    hid.previous_report = report;
                }
                XHCI_INTERRUPT_IN_DEQUEUE_INDEX[index] += 1;
                if XHCI_INTERRUPT_IN_DEQUEUE_INDEX[index] >= 31 {
                    XHCI_INTERRUPT_IN_DEQUEUE_INDEX[index] = 0;
                    XHCI_INTERRUPT_IN_CYCLE[index] = !XHCI_INTERRUPT_IN_CYCLE[index];
                }
                prime_interrupt_in_transfer(controller, hid)?;
            }
            return Ok(());
        }
        core::hint::spin_loop();
        count += 1;
    }
    Ok(())
}

pub fn decode_boot_keyboard_report(bytes: &[u8]) -> Option<HidBootKeyboardReport> {
    if bytes.len() < 8 {
        return None;
    }
    Some(HidBootKeyboardReport {
        modifiers: bytes[0],
        reserved: bytes[1],
        keys: [bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]],
    })
}

unsafe fn validate_root_hub_register_access(controller: XhciController) -> Result<(), UsbKbdError> {
    if controller.max_ports == 0 {
        return Err(UsbKbdError::InvalidMmio("root hub reports zero ports"));
    }
    let last_port = controller.max_ports - 1;
    let _ = mmio_read32(controller.op as *mut u8, portsc_offset(0));
    let _ = mmio_read32(controller.op as *mut u8, portsc_offset(last_port));
    Ok(())
}

unsafe fn port_connected(controller: XhciController, port: u8) -> Result<bool, UsbKbdError> {
    let portsc = mmio_read32(controller.op as *mut u8, portsc_offset(port));
    Ok(portsc & PORTSC_CCS != 0)
}

unsafe fn reset_connected_port(controller: XhciController, port: u8) -> Result<(), UsbKbdError> {
    let registers = XhciRegisters {
        op: controller.op as *mut u8,
        max_ports: controller.max_ports,
    };
    reset_port(registers, port)
}

unsafe fn read_port_speed(controller: XhciController, port: u8) -> u8 {
    let portsc = mmio_read32(controller.op as *mut u8, portsc_offset(port));
    ((portsc >> 10) & 0x0f) as u8
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
pub struct XhciTrb {
    pub parameter: u64,
    pub status: u32,
    pub control: u32,
}

impl XhciTrb {
    pub const fn new(parameter: u64, status: u32, trb_type: u8, cycle: bool) -> Self {
        let mut control = (trb_type as u32) << 10;
        if cycle {
            control |= 1;
        }
        Self {
            parameter,
            status,
            control,
        }
    }

    pub const fn trb_type(self) -> u8 {
        ((self.control >> 10) & 0x3f) as u8
    }

    pub const fn cycle(self) -> bool {
        self.control & 1 != 0
    }

    pub const fn words(self) -> [u32; 4] {
        [
            self.parameter as u32,
            (self.parameter >> 32) as u32,
            self.status,
            self.control,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XhciRingCursor {
    index: usize,
    cycle: bool,
    capacity: usize,
}

impl XhciRingCursor {
    pub const fn new(capacity: usize) -> Self {
        Self {
            index: 0,
            cycle: true,
            capacity,
        }
    }

    pub const fn index(self) -> usize {
        self.index
    }
    pub const fn cycle(self) -> bool {
        self.cycle
    }

    pub fn advance(&mut self) {
        if self.capacity == 0 {
            return;
        }
        self.index += 1;
        if self.index >= self.capacity {
            self.index = 0;
            self.cycle = !self.cycle;
        }
    }
}

pub const TRB_TYPE_NOOP_COMMAND: u8 = 23;
pub const TRB_TYPE_ENABLE_SLOT_COMMAND: u8 = 9;
pub const TRB_TYPE_ADDRESS_DEVICE_COMMAND: u8 = 11;
pub const TRB_TYPE_CONFIGURE_ENDPOINT_COMMAND: u8 = 12;
pub const TRB_TYPE_COMMAND_COMPLETION_EVENT: u8 = 33;
pub const TRB_TYPE_TRANSFER_EVENT: u8 = 32;
pub const TRB_TYPE_PORT_STATUS_CHANGE_EVENT: u8 = 34;

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

    pub const fn is_bulk_in(self) -> bool {
        self.address & 0x80 != 0 && self.attributes & 0x03 == 0x02
    }

    pub const fn is_bulk_out(self) -> bool {
        self.address & 0x80 == 0 && self.attributes & 0x03 == 0x02
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsbConfigurationScan {
    pub total_length: u16,
    pub configuration_value: u8,
    pub hid_boot_keyboard: Option<UsbInterfaceDescriptor>,
    pub interrupt_in: Option<UsbEndpointDescriptor>,
    pub mass_storage_bot: Option<UsbInterfaceDescriptor>,
    pub bulk_in: Option<UsbEndpointDescriptor>,
    pub bulk_out: Option<UsbEndpointDescriptor>,
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
        configuration_value: bytes[5],
        hid_boot_keyboard: None,
        interrupt_in: None,
        mass_storage_bot: None,
        bulk_in: None,
        bulk_out: None,
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
                if interface.class == 0x08
                    && interface.subclass == 0x06
                    && interface.protocol == 0x50
                {
                    scan.mass_storage_bot = Some(interface);
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
                if endpoint.is_bulk_in() {
                    scan.bulk_in = Some(endpoint);
                }
                if endpoint.is_bulk_out() {
                    scan.bulk_out = Some(endpoint);
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
    fn descriptor_parser_finds_mass_storage_bot_bulk_endpoints() {
        let config = [
            9, 2, 32, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 2, 0x08, 0x06, 0x50, 0, 7, 5, 0x81, 0x02,
            64, 0, 0, 7, 5, 0x02, 0x02, 64, 0, 0,
        ];
        let scan = scan_configuration_descriptor(&config).unwrap();
        assert!(scan.mass_storage_bot.is_some());
        assert!(scan.bulk_in.unwrap().is_bulk_in());
        assert!(scan.bulk_out.unwrap().is_bulk_out());
    }

    #[test]
    fn descriptor_parser_rejects_non_advancing_descriptor() {
        let malformed = [9, 2, 11, 0, 1, 1, 0, 0x80, 50, 0, 4];
        assert_eq!(
            scan_configuration_descriptor(&malformed),
            Err(DriverError::DescriptorMalformed)
        );
    }

    #[test]
    fn trb_packing_preserves_type_and_cycle() {
        let trb = XhciTrb::new(0x1122_3344_5566_7788, 8, TRB_TYPE_NOOP_COMMAND, true);
        assert_eq!(trb.words()[0], 0x5566_7788);
        assert_eq!(trb.words()[1], 0x1122_3344);
        assert_eq!(trb.trb_type(), TRB_TYPE_NOOP_COMMAND);
        assert!(trb.cycle());
    }

    #[test]
    fn ring_cursor_toggles_cycle_on_wrap() {
        let mut cursor = XhciRingCursor::new(2);
        assert_eq!(cursor.index(), 0);
        assert!(cursor.cycle());
        cursor.advance();
        assert_eq!(cursor.index(), 1);
        assert!(cursor.cycle());
        cursor.advance();
        assert_eq!(cursor.index(), 0);
        assert!(!cursor.cycle());
    }

    #[test]
    fn xhci_class_match_requires_programming_interface() {
        assert!(is_xhci_class(0x0c, 0x03, 0x30));
        assert!(!is_xhci_class(0x0c, 0x03, 0x20));
    }
}
