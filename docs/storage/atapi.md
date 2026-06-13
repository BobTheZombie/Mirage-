# AHCI ATAPI

AHCI port scan now distinguishes ATAPI signatures (`0xeb140101`) from SATA disks and logs honest ATAPI detection. Optical media is not faked as Online: until packet transport and SCSI media probing are wired, Optical Disk is Skipped with an explicit reason.

Next steps are IDENTIFY PACKET DEVICE, SCSI INQUIRY, READ CAPACITY(10), and READ(10), followed by read-only `atapi0` registration when media is present.
