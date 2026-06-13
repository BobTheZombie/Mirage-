# MBR Parser

The MBR parser reads LBA 0, requires the `0x55AA` signature, parses four primary entries, skips empty entries, detects protective GPT type `0xEE`, and rejects out-of-range partition extents.

Extended partitions are not implemented in this milestone.
