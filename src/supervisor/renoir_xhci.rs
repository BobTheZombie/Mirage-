//! Renoir-specific supervisor policy wrapper for the AMD xHCI controller.

pub use super::usb::{SupervisorXhciDevice, XhciOwnershipState, XhciPolicyDecision};

pub const AMD_VENDOR_ID: u16 = 0x1022;
pub const AMD_ATI_VENDOR_ID: u16 = 0x1002;

pub const fn is_amd_xhci_vendor(vendor_id: u16) -> bool {
    vendor_id == AMD_VENDOR_ID || vendor_id == AMD_ATI_VENDOR_ID
}
