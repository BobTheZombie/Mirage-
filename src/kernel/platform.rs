//! Platform discovery logging glue.
//!
//! Platform discovery records hardware presence before Mirage-dispatch-rs binds,
//! starts, or reports any driver/service lifecycle state.

use mirage_platform::{PlatformDevice, PlatformDeviceKind, PlatformLocation, PlatformRegistry};

pub fn register_platform_device<const CAPACITY: usize>(
    registry: &mut PlatformRegistry<CAPACITY>,
    device: PlatformDevice,
) {
    if let Ok(true) = registry.register(device) {
        platform_device_found(device);
    }
}

pub fn platform_device_found(device: PlatformDevice) {
    crate::kprint!("[Platform] device found: \"{}\", location=", device.name);
    write_location(device.location);
    crate::kprint!(", id=");
    write_hardware_id(device);
    crate::kprintln!();
}

fn write_location(location: PlatformLocation) {
    match location {
        PlatformLocation::CpuId => {
            crate::kprint!("cpuid");
        }
        PlatformLocation::Pci {
            bus,
            device,
            function,
        } => {
            crate::kprint!("pci {:02x}:{:02x}.{}", bus, device, function);
        }
        PlatformLocation::IoPort { base } => {
            if base == 0x60 {
                crate::kprint!("i8042");
            } else {
                crate::kprint!("io 0x{:x}", base);
            }
        }
        PlatformLocation::AcpiTable(signature) => {
            crate::kprint!("acpi {}", signature);
        }
        PlatformLocation::Usb { bus, port } => {
            crate::kprint!("usb {}:{}", bus, port);
        }
        PlatformLocation::Unknown => {
            crate::kprint!("unknown");
        }
    }
}

fn write_hardware_id(device: PlatformDevice) {
    match device.kind {
        PlatformDeviceKind::Cpu => {
            crate::kprint!(
                "AuthenticAMD family=0x{:x} model=0x{:x} stepping=0x{:x}",
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
                crate::kprint!("unknown");
            }
        },
    }
}
