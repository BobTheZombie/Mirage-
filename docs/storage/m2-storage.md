# M.2-Capable Storage Path

Mirage reports an “M.2-capable storage path” as an abstraction over two possible real storage transports:

- M.2 NVMe SSD → PCIe NVMe driver
- M.2 SATA SSD → AHCI/SATA driver

Rules:

- If NVMe is online, the M.2-capable path is online through NVMe.
- If AHCI/SATA disk is online, the M.2-capable path is online through SATA/AHCI.
- If no storage controller is present, the path is skipped.
- If a controller is detected but no block device becomes online, the path fails.
- Mirage must not claim a physical M.2 slot unless ACPI/SMBIOS proves it.
