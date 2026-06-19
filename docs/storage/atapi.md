# AHCI ATAPI

AHCI port scan now distinguishes ATAPI signatures (`0xeb140101`) from SATA disks and logs honest ATAPI detection. ATAPI presence is recorded as detected, but the ATAPI boot phase remains a non-online terminal state while packet media probing is not enabled. Optical media is not faked as Online: until packet transport and SCSI media probing are wired, Optical Disk is Skipped with an explicit reason.

Next steps are IDENTIFY PACKET DEVICE, SCSI INQUIRY, READ CAPACITY(10), and READ(10), followed by read-only `atapi0` registration when media is present. `BootPhase::OpticalDisk` must remain Skipped until those commands are implemented and the read-only `atapi0` device is registered.
