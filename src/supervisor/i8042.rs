//! Supervisor policy view for the lower-kernel i8042 controller.
//!
//! The supervisor records ownership and routing facts only. Raw access to ports
//! 0x60/0x64 remains exclusively in `arch::x86_64::i8042`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct I8042PolicyFacts {
    pub controller: &'static str,
    pub keyboard_irq: u8,
    pub data_port_owned_by_kernel: u16,
    pub command_port_owned_by_kernel: u16,
    pub supervisor_reads_ports: bool,
}

impl I8042PolicyFacts {
    pub const fn dell_inspiron_5505() -> Self {
        Self {
            controller: "i8042",
            keyboard_irq: 1,
            data_port_owned_by_kernel: 0x60,
            command_port_owned_by_kernel: 0x64,
            supervisor_reads_ports: false,
        }
    }
}
