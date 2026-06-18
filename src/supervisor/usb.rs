//! Supervisor-owned USB/xHCI policy records.
//!
//! This module intentionally contains no MMIO, DMA, TRB, or MTSS queue access.
//! It records policy decisions and lower-kernel status events so the supervisor
//! can approve ownership, visibility, and one bounded recovery attempt.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XhciOwnershipState {
    Discovered,
    Mapped,
    Initialized,
    Running,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XhciPolicyDecision {
    ApproveInitialization,
    ApproveUserspaceVisibility,
    ApproveHidRouting,
    ApproveOneResetRetry,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupervisorXhciDevice {
    pub pci_device: u64,
    pub vendor_id: u16,
    pub device_id: u16,
    pub mmio_base: u64,
    pub mmio_length: u64,
    pub irq_line: Option<u16>,
    pub state: XhciOwnershipState,
    pub reset_retries_used: u8,
    pub last_reason: &'static str,
}

impl SupervisorXhciDevice {
    pub const fn discovered(pci_device: u64, vendor_id: u16, device_id: u16) -> Self {
        Self {
            pci_device,
            vendor_id,
            device_id,
            mmio_base: 0,
            mmio_length: 0,
            irq_line: None,
            state: XhciOwnershipState::Discovered,
            reset_retries_used: 0,
            last_reason: "AMD XHCI [DETECTED]",
        }
    }

    pub const fn approve(&self, decision: XhciPolicyDecision) -> bool {
        match decision {
            XhciPolicyDecision::ApproveInitialization => {
                matches!(self.state, XhciOwnershipState::Discovered | XhciOwnershipState::Mapped)
            }
            XhciPolicyDecision::ApproveUserspaceVisibility => {
                matches!(self.state, XhciOwnershipState::Initialized | XhciOwnershipState::Running)
            }
            XhciPolicyDecision::ApproveHidRouting => {
                matches!(self.state, XhciOwnershipState::Running)
            }
            XhciPolicyDecision::ApproveOneResetRetry => self.reset_retries_used == 0,
            XhciPolicyDecision::Deny => false,
        }
    }

    pub fn note_mapped(&mut self, mmio_base: u64, mmio_length: u64, irq_line: Option<u16>) {
        self.mmio_base = mmio_base;
        self.mmio_length = mmio_length;
        self.irq_line = irq_line;
        self.state = XhciOwnershipState::Mapped;
        self.last_reason = "AMD XHCI [STARTED] mapped by lower kernel";
    }

    pub fn note_initialized(&mut self) {
        self.state = XhciOwnershipState::Initialized;
        self.last_reason = "AMD XHCI [STARTED] controller initialized; event path pending";
    }

    pub fn note_running_event_path(&mut self) {
        self.state = XhciOwnershipState::Running;
        self.last_reason = "AMD XHCI [ONLINE] controller running with event path";
    }

    pub fn note_failed(&mut self, reason: &'static str) {
        self.state = XhciOwnershipState::Failed;
        self.last_reason = reason;
    }

    pub fn consume_reset_retry(&mut self) -> bool {
        if self.reset_retries_used == 0 {
            self.reset_retries_used = 1;
            true
        } else {
            false
        }
    }
}
