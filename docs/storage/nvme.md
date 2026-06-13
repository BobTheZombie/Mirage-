# NVMe Storage

NVMe controllers are discovered only from the Platform Registry using PCI class `0x01`, subclass `0x08`, prog-if `0x02`.

Boot status rules:

- absent controller: `NVMe -> Skipped`
- present controller: `NVMe -> Detected -> Started -> Online/Failed`
- `Online` requires at least one registered namespace such as `nvme0n1`
- writes are disabled unless policy explicitly enables them
- controller/admin/IO queue waits must be bounded

M.2 PCIe SSDs use this NVMe path. M.2 describes the connector/form factor, not the protocol.
