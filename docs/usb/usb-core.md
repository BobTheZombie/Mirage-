# Mirage USB Core

The current USB Core is an xHCI-backed bus-manager skeleton. It scans root ports,
resets connected ports with timeouts, and records fixed-capacity device records.
It must not infer class drivers from port presence.

Descriptor parsing is defensive: device descriptors require at least 18 bytes,
configuration descriptors require at least 9 bytes, every descriptor must have
`bLength >= 2`, and iteration never reads past `wTotalLength`.

Next work: Enable Slot, Address Device, endpoint-0 context setup, GET_DESCRIPTOR,
SET_CONFIGURATION, and safe transfer completion polling.
