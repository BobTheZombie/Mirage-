# Partition Layer

The partition layer sits above `BlockDevice` and parses MBR or GPT from an already-registered whole block device. Empty or absent partition tables are Skipped, not Failed.

Supported now:

- primary MBR entries
- protective MBR detection
- GPT primary header validation
- GPT header CRC32
- GPT partition entry array CRC32
- UTF-16LE GPT partition names converted to ASCII-compatible bytes

Extended MBR partitions and backup GPT fallback are documented future work.
