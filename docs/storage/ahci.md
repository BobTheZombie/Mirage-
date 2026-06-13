# AHCI / SATA Storage

AHCI controllers are discovered only from the Platform Registry using PCI class `0x01`, subclass `0x06`, prog-if `0x01`.

Boot status rules:

- absent controller: `AHCI -> Skipped`
- present controller: `AHCI -> Detected -> Started -> Online/Failed`
- `Online` requires at least one registered `sataN` block device
- no infinite waits; command-slot and completion polling must be bounded
- no writes during discovery or root probing

M.2 SATA SSDs use this AHCI/SATA path. Mirage must not call this an M.2 protocol.

## Port Classification and Early DMA Path

AHCI setup validates PCI BAR5, maps ABAR as device MMIO, reads HBA registers with volatile accesses, enables AHCI mode, scans implemented ports, decodes `PxSSTS` DET/IPM, and classifies `PxSIG` as SATA, ATAPI, SEMB, port multiplier, or unknown.

SATA disks use bounded command-engine stop/start, DMA frame setup, IDENTIFY DEVICE, and READ DMA EXT. Writes remain disabled by default and return read-only unless future kernel write policy and rw mount state explicitly enable them. ATAPI is detected honestly and does not fake Optical Disk Online before packet/media probing is complete.
