//! Platform discovery logging glue.
//!
//! Platform discovery records hardware presence before Mirage-dispatch-rs binds,
//! starts, or reports any driver/service lifecycle state.

use mirage_device_db::{
    lookup_cpu_amd, lookup_cpu_intel, lookup_input_device, lookup_pci_class, lookup_pci_device,
};
use mirage_platform::{PlatformDevice, PlatformDeviceKind, PlatformLocation, PlatformRegistry};

use crate::kernel::boot_phase::{boot_phase_detected, boot_phase_state, BootPhase, PhaseState};

pub fn register_platform_device<const CAPACITY: usize>(
    registry: &mut PlatformRegistry<CAPACITY>,
    device: PlatformDevice,
) {
    if let Ok(true) = registry.register(device) {
        platform_device_found(device);
        mark_boot_phase_detected(device);
    }
}

pub fn platform_device_found(device: PlatformDevice) {
    crate::kprintln!("[Platform] Device found:");
    crate::kprintln!("    {}", display_name(device));
    crate::kprint!("    Location: ");
    write_location(device.location);
    crate::kprint!("\n    ID: ");
    write_hardware_id(device);
    crate::kprintln!();

    match device.kind {
        PlatformDeviceKind::Cpu => write_cpu_descriptor(device),
        PlatformDeviceKind::I8042 | PlatformDeviceKind::Input => write_input_descriptor(device),
        _ if matches!(device.location, PlatformLocation::Pci { .. }) => {
            write_pci_descriptor(device)
        }
        _ => {}
    }
}

fn display_name(device: PlatformDevice) -> &'static str {
    match (device.vendor_id, device.device_id) {
        (Some(vendor), Some(device_id)) => lookup_pci_device(vendor, device_id)
            .map(|entry| entry.name)
            .unwrap_or(device.name),
        _ => device.name,
    }
}

fn write_pci_descriptor(device: PlatformDevice) {
    if let (Some(class), Some(subclass), Some(prog_if)) =
        (device.class_code, device.subclass, device.prog_if)
    {
        match lookup_pci_class(class, subclass, prog_if) {
            Some(class_info) => {
                crate::kprintln!(
                    "    Class: {} (class=0x{:02x}, subclass=0x{:02x}, prog-if=0x{:02x})",
                    class_info.name,
                    class,
                    subclass,
                    prog_if
                );
                write_driver_hint(pci_device_hint(device).or(class_info.driver_hint));
            }
            None => {
                crate::kprintln!(
                    "    Class: unknown (class=0x{:02x}, subclass=0x{:02x}, prog-if=0x{:02x})",
                    class,
                    subclass,
                    prog_if
                );
                write_driver_hint(pci_device_hint(device));
            }
        }
    } else {
        write_driver_hint(pci_device_hint(device));
    }
}

fn pci_device_hint(device: PlatformDevice) -> Option<&'static str> {
    match (device.vendor_id, device.device_id) {
        (Some(vendor), Some(device_id)) => {
            lookup_pci_device(vendor, device_id).and_then(|entry| entry.driver_hint)
        }
        _ => None,
    }
}

fn write_cpu_descriptor(device: PlatformDevice) {
    let (Some(family), Some(model), Some(stepping)) =
        (device.class_code, device.subclass, device.prog_if)
    else {
        return;
    };

    let descriptor = lookup_cpu_amd(family as u16, model as u16, stepping)
        .or_else(|| lookup_cpu_intel(family as u16, model as u16, stepping));

    if let Some(cpu) = descriptor {
        if let Some(codename) = cpu.codename {
            crate::kprintln!("    Codename: {}", codename);
        }
        write_driver_hints(cpu.driver_hints);
    }
}

fn write_input_descriptor(device: PlatformDevice) {
    let descriptor = match device.kind {
        PlatformDeviceKind::I8042 => lookup_input_device("i8042"),
        PlatformDeviceKind::Input => lookup_input_device("input"),
        _ => None,
    };

    if let Some(input) = descriptor {
        write_driver_hints(input.driver_hints);
    }
}

fn write_driver_hint(hint: Option<&'static str>) {
    match hint {
        Some(hint) => crate::kprintln!("    Driver hint: {}", hint),
        None => crate::kprintln!("    Driver hint: none"),
    }
}

fn write_driver_hints(hints: &[&'static str]) {
    crate::kprint!("    Driver hint: ");
    if hints.is_empty() {
        crate::kprintln!("none");
        return;
    }
    let mut index = 0;
    while index < hints.len() {
        if index != 0 {
            crate::kprint!(", ");
        }
        crate::kprint!("{}", hints[index]);
        index += 1;
    }
    crate::kprintln!();
}

fn write_location(location: PlatformLocation) {
    match location {
        PlatformLocation::CpuId => {
            crate::kprint!("CPUID");
        }
        PlatformLocation::Pci {
            bus,
            device,
            function,
        } => {
            crate::kprint!("PCI {:02x}:{:02x}.{}", bus, device, function);
        }
        PlatformLocation::IoPort { base } => {
            if base == 0x60 {
                crate::kprint!("I8042");
            } else {
                crate::kprint!("IO 0x{:x}", base);
            }
        }
        PlatformLocation::AcpiTable(signature) => {
            crate::kprint!("ACPI {}", signature);
        }
        PlatformLocation::Usb { bus, port } => {
            crate::kprint!("USB {}:{}", bus, port);
        }
        PlatformLocation::Unknown => {
            crate::kprint!("Unknown");
        }
    }
}

fn write_hardware_id(device: PlatformDevice) {
    match device.kind {
        PlatformDeviceKind::Cpu => {
            crate::kprint!(
                "CPUID family=0x{:x} model=0x{:x} stepping=0x{:x}",
                device.class_code.unwrap_or(0),
                device.subclass.unwrap_or(0),
                device.prog_if.unwrap_or(0)
            );
        }
        PlatformDeviceKind::I8042 => {
            crate::kprint!("0x60/0x64");
        }
        _ => match (device.vendor_id, device.device_id) {
            (Some(vendor), Some(device_id)) => {
                crate::kprint!("{:04x}:{:04x}", vendor, device_id);
            }
            _ => {
                crate::kprint!("Unknown");
            }
        },
    }
}

fn mark_boot_phase_detected(device: PlatformDevice) {
    match device.kind {
        PlatformDeviceKind::Cpu => {
            detect_if_registered(BootPhase::Amd64Cpu);
            if device.name.contains("Ryzen") {
                detect_if_registered(BootPhase::RyzenCpu);
            }
        }
        PlatformDeviceKind::Storage => {
            if device.class_code == Some(0x01)
                && device.subclass == Some(0x08)
                && device.prog_if == Some(0x02)
            {
                detect_if_registered(BootPhase::Nvme);
            } else if device.class_code == Some(0x01)
                && device.subclass == Some(0x06)
                && device.prog_if == Some(0x01)
            {
                detect_if_registered(BootPhase::Ahci);
            }
        }
        PlatformDeviceKind::Display => {
            if device.vendor_id == Some(0x1002) && matches!(device.device_id, Some(0x1636 | 0x1638))
            {
                detect_if_registered(BootPhase::AmdGpuRenoir);
            }
        }
        PlatformDeviceKind::Usb => {
            if device.vendor_id == Some(0x1022)
                && device.class_code == Some(0x0c)
                && device.subclass == Some(0x03)
                && device.prog_if == Some(0x30)
            {
                detect_if_registered(BootPhase::AmdXhci);
                detect_if_registered(BootPhase::Xhci);
            }
        }
        PlatformDeviceKind::I8042 => detect_if_registered(BootPhase::I8042),
        PlatformDeviceKind::Acpi => detect_if_registered(BootPhase::AcpiTables),
        _ => {}
    }
}

fn detect_if_registered(phase: BootPhase) {
    if boot_phase_state(phase) != PhaseState::Unregistered {
        boot_phase_detected(phase);
    }
}
