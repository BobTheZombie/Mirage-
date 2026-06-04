# Mirage NVMe Hardware Status

## Current real support level

Mirage does not currently provide production-ready NVMe real hardware support.
The default NVMe path is a mock controller and in-memory namespace block device.
The `hw-nvme` feature adds a hardware-shaped controller skeleton, but it still
uses modeled register state and in-memory namespace storage instead of performing
real NVMe command submission, DMA, or completion processing against a controller.

Do not claim real NVMe support until Mirage initializes an NVMe controller and
performs verified block I/O against a real NVMe device or a validated NVMe
emulator.

## What code is actually implemented

* `NvmeHardwareResources` records the supervisor-granted PCI device, MMIO range,
  DMA region, and IRQ line expected by an NVMe controller.
* `MockNvmeController` creates mock identify data, mock admin/I/O queues, and
  mock namespaces.
* `MockNvmeBlockDevice` implements the common block interface by reading and
  writing an in-memory namespace after checking the required capabilities.
* `hw-nvme` adds types for controller registers, status, submission/completion
  queues, an admin queue, I/O queue descriptors, namespace descriptors, and an
  `NvmeHardwareController` wrapper.
* The hardware skeleton validates PCI class/subclass/prog-if information and
  extracts a PCI BAR-shaped MMIO region from caller-provided PCI metadata.

## What remains mocked

* Controller register reads/writes are modeled in Rust state; the driver does
  not map real controller registers for command execution.
* Identify data is still generated with mock data.
* Namespace block data is held in memory, not on an NVMe device.
* PRP/SGL setup, DMA-safe command/data buffers, doorbell writes, completion
  queue polling against hardware, and MSI/MSI-X interrupts are not implemented.
* Queue allocation and lifecycle are placeholders suitable for architecture
  tests, not a real queue-pair implementation.
* Controller reset, shutdown notification, error-log handling, namespaces beyond
  the default placeholder, multipath, write cache policy, and power management
  remain TODOs.

## Intended hardware or emulator target

The first target should be a QEMU standard NVMe PCI controller attached to a
scratch disk image. Validation should begin with polling-only read commands and
known-pattern integrity checks before writes are enabled. Real NVMe SSD testing
must wait until DMA, IOMMU/resource isolation, controller reset, and error
recovery are implemented and documented.

## Known safety limitations

* No real NVMe DMA safety exists yet; incorrect PRP/SGL programming could corrupt
  memory when real hardware access is added.
* The write path is safe only for the in-memory mock namespace today.
* Capability checks verify Mirage authority metadata, but do not yet guarantee
  hardware isolation or revocation of in-flight DMA.
* There is no timeout/recovery path proven against a wedged real controller.
* The skeleton should not be used with user data or non-scratch disks.

## TODO roadmap

1. Add a documented QEMU NVMe validation recipe.
2. Replace modeled register state with checked MMIO access where permitted by
   supervisor-granted capabilities.
3. Implement controller reset/disable/enable sequencing from the NVMe spec.
4. Allocate admin queue memory from DMA-safe regions and wire doorbell writes.
5. Submit Identify Controller and Identify Namespace commands against a validated
   emulator and verify completion status.
6. Implement one polling I/O queue for read-only block reads.
7. Add bounded writes only for scratch media after read integrity tests pass.
8. Add MSI/MSI-X interrupt routing through capability-scoped IRQ ownership.
9. Add service crash cleanup: quiesce queues, revoke DMA/MMIO/IRQ capabilities,
   reset the controller, and re-register namespaces through the supervisor.
