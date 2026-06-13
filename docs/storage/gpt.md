# GPT Parser

The GPT parser requires a protective MBR, reads the primary GPT header at LBA 1, validates the `EFI PART` signature, checks header size, validates the header CRC32 with the CRC field zeroed, validates the partition-entry-array CRC32, and checks each used partition against the usable LBA range.

Backup GPT fallback is future work.
