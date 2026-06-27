#![no_std]

//! Static hardware descriptor database for Mirage diagnostics and driver hints.
//!
//! The database is generated at build time by `tools/gen-device-db` from the
//! checked-in descriptor source files. Runtime lookup functions only scan static
//! slices: they do not allocate, parse TOML, or panic. Hardware binding and
//! service launch policy must still be mediated by Mirage capability checks and
//! the Supervisor; these descriptors are identity metadata and driver hints.

mod generated;

pub use generated::{
    lookup_block_kind, lookup_char_kind, lookup_cpu_amd, lookup_cpu_intel, lookup_input_kind,
    lookup_pci_class, lookup_pci_device, lookup_pci_vendor, lookup_usb_class, lookup_usb_device,
    lookup_usb_vendor, BlockDeviceDescriptor, CharDeviceDescriptor, ChipsetDescriptor, CpuInfo,
    InputDeviceDescriptor, PciClassDescriptor, PciDeviceDescriptor, PciVendorDescriptor,
    UsbClassDescriptor, UsbDeviceDescriptor, UsbVendorDescriptor,
};

/// Bus-neutral vendor metadata shape used by PCI and USB descriptors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VendorDescriptor {
    pub id: u16,
    pub name: &'static str,
    pub aliases: &'static [&'static str],
}

#[cfg(test)]
mod tests {
    use super::{
        lookup_block_kind, lookup_char_kind, lookup_cpu_amd, lookup_cpu_intel, lookup_input_kind,
        lookup_pci_class, lookup_pci_device, lookup_pci_vendor, lookup_usb_class,
        lookup_usb_device, lookup_usb_vendor,
    };

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
    fn looks_up_pci_and_usb_descriptors() {
        let vendor = lookup_pci_vendor(0x1002).expect("AMD PCI vendor");
        assert_eq!(vendor.name, "AMD");

        let pci = lookup_pci_device(0x1002, 0x1636).expect("Renoir GPU device entry");
        assert_eq!(pci.name, "AMD Radeon Renoir Graphics");
        assert_eq!(pci.driver_hint, Some("amdgpu/displayd"));

        let usb_vendor = lookup_usb_vendor(0x046d).expect("Logitech USB vendor");
        assert_eq!(usb_vendor.name, "Logitech");

        let usb = lookup_usb_device(0x046d, 0xc31c).expect("Logitech keyboard");
        assert_eq!(usb.driver_hint, Some("hid/inputd"));
    }

    #[test]
    fn class_lookup_prefers_exact_prog_if_before_generic_entry() {
        let nvme = lookup_pci_class(0x01, 0x08, 0x02).expect("NVMe class entry");
        assert_eq!(nvme.name, "NVMe Controller");
        assert_eq!(nvme.driver_hint, Some("nvme/storaged"));

        let hid = lookup_usb_class(0x03, 0x01, 0x01).expect("generic HID class entry");
        assert_eq!(hid.name, "Human Interface Device");
    }

    #[test]
    fn looks_up_block_char_and_input_driver_hints() {
        let sata = lookup_block_kind("ahci-disk").expect("AHCI disk block descriptor");
        assert_eq!(sata.name, "AHCI SATA Disk");
        assert_eq!(sata.driver_hint, "ahci/storaged");
        assert_eq!(sata.default_block_size, 512);

        let serial = lookup_char_kind("serial-console").expect("serial char descriptor");
        assert_eq!(serial.driver_hints, &["uart16550", "console"]);

        let keyboard =
            lookup_input_kind("usb-hid-keyboard").expect("USB keyboard input descriptor");
        assert_eq!(keyboard.driver_hints, &["xhci", "usbd", "hid", "inputd"]);
    }
}
