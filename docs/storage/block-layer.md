# Mirage Block Layer

The block layer is the storage-transport-neutral contract used by QFS and storage services.

- Devices advertise `BlockDeviceInfo` including id, name, kind, block size, block count, read-only state, and write-cache state.
- `BlockRange` validates non-empty LBA ranges, overflow, and device bounds.
- Reads and writes validate exact buffer lengths before touching a backend.
- `FixedBlockDeviceRegistry` is fixed-capacity and requires no heap allocation for registration, lookup, enumeration, unregister, read, write, and flush dispatch.
- Drivers must not register fake online devices. The only built-in fallback must be explicitly named `BuiltInBlockQfs`.
