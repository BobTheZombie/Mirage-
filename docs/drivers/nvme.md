# NVMe Driver

Mirage discovers NVMe with PCI class `0x01`, subclass `0x08`, prog-if `0x02`. NVMe is an additional block-device path and must not replace or regress the existing AHCI/ATAPI boot path unless root selection is explicitly configured.

Bring-up order:

1. Match PCI class/subclass/prog-if.
2. Validate BAR0/BAR1 MMIO through the common PCI/MMIO layer.
3. Read CAP/VS/CC/CSTS.
4. Disable the controller with a bounded `CSTS.RDY` wait.
5. Allocate DMA-aligned admin submission and completion queues.
6. Program AQA/ASQ/ACQ.
7. Enable the controller with a bounded ready wait.
8. Submit Identify Controller and Identify Namespace commands.
9. Register a read-only block device only after namespace identify succeeds.

Known limitations: early hardware paths remain polling-first and read-only; MSI/MSI-X and write support are future work.
