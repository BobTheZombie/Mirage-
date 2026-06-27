#![no_std]

//! Small, static PCI identity database for Mirage diagnostics.
//!
//! This crate intentionally provides names and driver hints only. Hardware
//! binding must continue to use raw PCI class/subclass/programming-interface
//! codes and capability-mediated driver policy rather than trusting strings.

mod generated;
pub use generated::{lookup_cpu_amd, lookup_cpu_intel, CpuInfo};

/// Known PCI vendor metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciVendorInfo {
    pub vendor_id: u16,
    pub name: &'static str,
}

/// Known PCI device metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciDeviceInfo {
    pub vendor_id: u16,
    pub device_id: u16,
    pub name: &'static str,
    pub driver_hint: Option<&'static str>,
}

/// PCI class-code metadata for generic diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciClassInfo {
    pub class: u8,
    pub subclass: u8,
    pub prog_if: Option<u8>,
    pub name: &'static str,
    pub driver_hint: Option<&'static str>,
}

const PCI_VENDORS: &[PciVendorInfo] = &[
    PciVendorInfo {
        vendor_id: 0x1002,
        name: "AMD",
    },
    PciVendorInfo {
        vendor_id: 0x1022,
        name: "AMD",
    },
    PciVendorInfo {
        vendor_id: 0x1234,
        name: "QEMU",
    },
    PciVendorInfo {
        vendor_id: 0x1af4,
        name: "VirtIO",
    },
    PciVendorInfo {
        vendor_id: 0x8086,
        name: "Intel",
    },
];

const PCI_DEVICES: &[PciDeviceInfo] = &[
    PciDeviceInfo {
        vendor_id: 0x1002,
        device_id: 0x1636,
        name: "AMD Radeon Renoir Graphics",
        driver_hint: Some("amdgpu/displayd"),
    },
    PciDeviceInfo {
        vendor_id: 0x1002,
        device_id: 0x1638,
        name: "AMD Radeon Renoir Graphics",
        driver_hint: Some("amdgpu/displayd"),
    },
    PciDeviceInfo {
        vendor_id: 0x1234,
        device_id: 0x1111,
        name: "QEMU VGA Display Controller",
        driver_hint: Some("displayd"),
    },
    PciDeviceInfo {
        vendor_id: 0x8086,
        device_id: 0x100e,
        name: "Intel 82540EM Gigabit Ethernet Controller",
        driver_hint: Some("netd"),
    },
    PciDeviceInfo {
        vendor_id: 0x8086,
        device_id: 0x2922,
        name: "Intel ICH9 AHCI Controller",
        driver_hint: Some("ahci/storaged"),
    },
    PciDeviceInfo {
        vendor_id: 0x8086,
        device_id: 0x29c0,
        name: "Intel Q35 Host Bridge",
        driver_hint: None,
    },
];

const PCI_CLASSES: &[PciClassInfo] = &[
    PciClassInfo {
        class: 0x01,
        subclass: 0x06,
        prog_if: Some(0x01),
        name: "AHCI Controller",
        driver_hint: Some("ahci/storaged"),
    },
    PciClassInfo {
        class: 0x01,
        subclass: 0x08,
        prog_if: Some(0x02),
        name: "NVMe Controller",
        driver_hint: Some("nvme/storaged"),
    },
    PciClassInfo {
        class: 0x02,
        subclass: 0x00,
        prog_if: None,
        name: "Ethernet Controller",
        driver_hint: Some("netd"),
    },
    PciClassInfo {
        class: 0x03,
        subclass: 0x00,
        prog_if: None,
        name: "VGA Display Controller",
        driver_hint: Some("displayd"),
    },
    PciClassInfo {
        class: 0x03,
        subclass: 0x80,
        prog_if: None,
        name: "Display Controller",
        driver_hint: Some("displayd"),
    },
    PciClassInfo {
        class: 0x06,
        subclass: 0x00,
        prog_if: None,
        name: "Host Bridge",
        driver_hint: None,
    },
    PciClassInfo {
        class: 0x0c,
        subclass: 0x03,
        prog_if: Some(0x30),
        name: "xHCI Controller",
        driver_hint: Some("xhci/usbd"),
    },
];

pub const fn lookup_pci_vendor(vendor_id: u16) -> Option<&'static PciVendorInfo> {
    let mut index = 0;
    while index < PCI_VENDORS.len() {
        let vendor = &PCI_VENDORS[index];
        if vendor.vendor_id == vendor_id {
            return Some(vendor);
        }
        index += 1;
    }
    None
}

pub const fn lookup_pci_device(vendor_id: u16, device_id: u16) -> Option<&'static PciDeviceInfo> {
    let mut index = 0;
    while index < PCI_DEVICES.len() {
        let device = &PCI_DEVICES[index];
        if device.vendor_id == vendor_id && device.device_id == device_id {
            return Some(device);
        }
        index += 1;
    }
    None
}

pub const fn lookup_pci_class(
    class: u8,
    subclass: u8,
    prog_if: u8,
) -> Option<&'static PciClassInfo> {
    let mut fallback = None;
    let mut index = 0;
    while index < PCI_CLASSES.len() {
        let entry = &PCI_CLASSES[index];
        if entry.class == class && entry.subclass == subclass {
            match entry.prog_if {
                Some(required) if required == prog_if => return Some(entry),
                None => fallback = Some(entry),
                _ => {}
            }
        }
        index += 1;
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::{lookup_cpu_amd, lookup_cpu_intel};
    use super::{lookup_pci_class, lookup_pci_device, lookup_pci_vendor};

    #[test]
    fn cpu_lookup_prefers_exact_stepping_before_model_fallback() {
        let exact = lookup_cpu_amd(0x17, 0x60, 0x1).expect("exact Renoir stepping entry");
        assert_eq!(exact.name, "AMD Ryzen 5 4500U");
        assert_eq!(exact.codename, Some("Renoir"));
        assert_eq!(
            exact.driver_hints,
            &["amd/renoir", "amdgpu/displayd", "xhci/usbd"]
        );

        let fallback = lookup_cpu_amd(0x17, 0x60, 0x2).expect("Renoir model fallback entry");
        assert_eq!(fallback.name, "AMD Ryzen 4000 Mobile APU");
    }

    #[test]
    fn cpu_lookup_uses_family_fallback_after_model_miss() {
        let amd = lookup_cpu_amd(0x19, 0xff, 0x0).expect("AMD family fallback");
        assert_eq!(amd.codename, Some("Zen family 19h"));

        let intel = lookup_cpu_intel(0x06, 0xff, 0x0).expect("Intel family fallback");
        assert_eq!(intel.name, "Intel 64 CPU");
    }

    #[test]
    fn looks_up_known_device_and_driver_hint() {
        let device = lookup_pci_device(0x1002, 0x1636).expect("Renoir GPU device entry");
        assert_eq!(device.name, "AMD Radeon Renoir Graphics");
        assert_eq!(device.driver_hint, Some("amdgpu/displayd"));
    }

    #[test]
    fn class_lookup_prefers_exact_prog_if_before_generic_entry() {
        let nvme = lookup_pci_class(0x01, 0x08, 0x02).expect("NVMe class entry");
        assert_eq!(nvme.name, "NVMe Controller");
        assert_eq!(nvme.driver_hint, Some("nvme/storaged"));
    }

    #[test]
    fn unknown_entries_return_none_for_conservative_fallbacks() {
        assert!(lookup_pci_vendor(0xffff).is_none());
        assert!(lookup_pci_device(0xffff, 0xffff).is_none());
        assert!(lookup_pci_class(0xff, 0xff, 0xff).is_none());
    }
}
