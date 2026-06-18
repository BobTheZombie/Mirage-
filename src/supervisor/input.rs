//! Supervisor-owned input policy facts.
//!
//! This module records policy-visible device facts handed up by lower-kernel
//! drivers. It intentionally contains no port I/O and cannot mutate IRQ handlers
//! or decoder state.

use crate::kernel::input::InputRawSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputRoutePolicy {
    DebugShell,
    KernelConsole,
    FutureUserspace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupervisorInputDevice {
    pub source: InputRawSource,
    pub device_type: &'static str,
    pub controller_type: &'static str,
    pub scancode_set: &'static str,
    pub irq_mode: bool,
    pub status: &'static str,
    pub debug_shell_route: bool,
    pub kernel_console_route: bool,
    pub userspace_route_pending: bool,
}

impl SupervisorInputDevice {
    pub const fn ps2_keyboard(
        scancode_set: &'static str,
        irq_mode: bool,
        status: &'static str,
    ) -> Self {
        Self {
            source: InputRawSource::Ps2,
            device_type: "internal-keyboard",
            controller_type: "i8042",
            scancode_set,
            irq_mode,
            status,
            debug_shell_route: true,
            kernel_console_route: true,
            userspace_route_pending: true,
        }
    }

    pub const fn approves(self, policy: InputRoutePolicy) -> bool {
        match policy {
            InputRoutePolicy::DebugShell => self.debug_shell_route,
            InputRoutePolicy::KernelConsole => self.kernel_console_route,
            InputRoutePolicy::FutureUserspace => self.userspace_route_pending,
        }
    }
}
