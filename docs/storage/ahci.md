# AHCI / SATA Storage

AHCI controllers are discovered only from the Platform Registry using PCI class `0x01`, subclass `0x06`, prog-if `0x01`.

Boot status rules:

- absent controller: `AHCI -> Skipped`
- present controller: `AHCI -> Detected -> Started -> Online/Failed`
- `Online` requires at least one registered `sataN` block device
- no infinite waits; command-slot and completion polling must be bounded
- no writes during discovery or root probing

M.2 SATA SSDs use this AHCI/SATA path. Mirage must not call this an M.2 protocol.
