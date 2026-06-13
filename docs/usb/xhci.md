# Mirage xHCI Bring-up

Mirage treats xHCI as an early kernel mechanism until a supervised `usbd.xhci`
service exists. Discovery comes from the Platform Registry, not from driver-local
PCI rescans.

Initialization sequence implemented today:

1. Select PCI class `0x0c`, subclass `0x03`, prog-if `0x30` from the registry.
2. Use registry BAR0 when available and fall back to PCI config BAR0 validation.
3. Enable PCI memory space and bus mastering.
4. Read CAPLENGTH, HCSPARAMS1, HCCPARAMS1, DBOFF, and RTSOFF.
5. Halt, reset, and run the controller with bounded waits.
6. Program static aligned DCBAA and command ring backing.
7. Keep event-ring backing reserved for the future interrupt/poll owner.

Known limitations: full command/event completion, transfer rings, contexts,
scratchpads, MSI/MSI-X, and interrupt ownership are not complete.
