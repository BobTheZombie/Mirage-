# AHCI full bring-up audit

## Scope

This audit covers the Mirage x86_64 boot path for PCI discovery, AHCI startup,
MMIO mapping, block registration, root filesystem policy, and kernel page-fault
handling.

## Current AHCI flow before the fix

1. The x86_64 platform probe scans PCI configuration space through CF8/CFC and
   records devices in the platform registry with PCI location, IDs, class codes,
   IRQ line, and decoded BAR metadata.
2. `initialize_storage_hardware` detects an AHCI controller when the registry has
   a storage PCI function with class `0x01`, subclass `0x06`, and prog-if `0x01`.
3. The old AHCI path enabled PCI command memory/bus-master bits and then took
   `PlatformDevice::mmio_bar(5)` as the AHCI ABAR.
4. The old AHCI path computed the HBA register pointer as `hhdm_offset + BAR5`.
5. The first volatile reads were HBA `CAP`, `GHC`, `PI`, and `VS` from that
   computed address.

## Exact point of crash

The observed faulting address, `0xffff8000febd5000`, matches an HHDM-style
translation of the AHCI ABAR physical address `0xfebd5000`. The crash happens on
or before the first AHCI HBA register read (`CAP` at offset `0x00`) because the
old driver dereferenced the computed HHDM virtual address without proving that
MMIO BAR5 was mapped into the kernel page tables.

The reported error code `0x0` decodes to a supervisor read of a non-present page,
which is consistent with a missing MMIO PTE rather than an AHCI protocol error.

## PCI BAR reading

The platform probe decodes BARs from endpoint PCI configuration space and stores
raw/base/type/prefetchability metadata in `PlatformPciBar`. It did not probe BAR
size during platform discovery. AHCI now re-reads/probes BAR5 size immediately
before MMIO use, restores the original BAR values, and logs the raw value,
physical base, type, width, prefetchability, size, and PCI command register
before/after enable.

## PCI command register enable path

The old AHCI path wrote command bits but did not log the command register before
and after, and did not verify that memory-space and bus-master bits stuck. The
new path reads command before, sets memory space and bus mastering, reads back,
and fails AHCI if either required bit remains clear.

## Platform Registry PCI records

The registry stores enough information to find AHCI and locate BAR5, but it is a
presence registry only. A platform record does not mean the driver is online, the
BAR is mapped, or a block device exists. AHCI lifecycle is now kept separate:
Detected -> Started -> Online/Failed, with SATA disk skipped when no disk is
present.

## MMIO mapping helpers and HHDM usage

Before the fix, AHCI used HHDM directly for MMIO (`hhdm + bar.base`). That is not
valid unless firmware/bootloader page tables actually map the MMIO range, and the
page fault demonstrates they did not.

The new central `src/kernel/mmio.rs` API maps MMIO by page-aligning the physical
base down, preserving the in-page offset in the returned virtual address,
allocating from a dedicated high-kernel MMIO virtual range, setting present,
writable, supervisor-only, no-execute, cache-disabled PTEs, and verifying the
page-table walk before returning. AHCI uses only the returned virtual address for
HBA register access.

HHDM remains used for CPU access to RAM-backed DMA buffers allocated from the
physical frame allocator; those pages are normal RAM and expected to be covered
by the bootloader HHDM.

## Page table mapper

The page-table mapper already supported frame-backed page-table growth and
mapping arbitrary kernel pages after physical-memory initialization. It did not
provide a central MMIO wrapper or a public diagnostic page-table walk. The fix
adds a public kernel page-table walk used by both the page-fault diagnostic and
AHCI MMIO verification diagnostics.

## AHCI driver

The old driver:

- did not explicitly map BAR5;
- did not validate BAR5 beyond `mmio_bar(5)`;
- read HBA registers through raw volatile pointers derived from HHDM;
- did bounded waits for command completion/port stop, but did not expose a
  central MMIO safety boundary;
- registered a global `sata0` state after IDENTIFY, but did not register a real
  kernel `DeviceDriver`/`BlockStorageDevice` for later root mounting.

The new driver:

- validates BAR5 and PCI command bits before MMIO;
- maps ABAR through `map_mmio`;
- verifies HBA register coverage before the first volatile read;
- reads and logs `CAP`, `GHC`, `PI`, and `VS`;
- enables AHCI mode with bounded reset-bit waits;
- scans implemented ports, decodes `PxSSTS`, classifies `PxSIG`, and skips cleanly
  when no SATA disk is present;
- allocates/programs command-list, received-FIS, command-table, and data DMA
  buffers for SATA ports;
- issues ATA IDENTIFY and parses model/sectors/sector size;
- exposes read-only `sata0` and rejects writes.

## Block layer registration

The early AHCI bring-up records `sata0` only after IDENTIFY succeeds. The x86_64
real-driver registration glue now registers an AHCI `BlockStorageDevice` named
`sata0` only when that state exists. This avoids fake online states and avoids a
block device descriptor for absent hardware.

## Root FS mount path

The current kernel `BootInfo` path does not expose a kernel command-line/root
selector to `mount_root_from_boot_sources`, so this change does not introduce a
fake `root=sata0` policy. The existing mount path still falls back to the built-in
QFS block device, so AHCI absence or SATA disk absence does not fail boot. A
future root selector can require `sata0` only when the boot command line is wired
into the kernel root policy.

## Page fault handler behavior

The old page-fault path printed diagnostics and returned, which could re-enter
the same kernel fault indefinitely. The new kernel-mode page-fault path prints one
diagnostic with CR2, error code, registers when available, decoded page-fault
bits, CR3, a page-table walk for CR2, and then halts through `cli/hlt` instead of
returning to the faulting instruction.

## What changed

- Added central MMIO mapping and verification in `src/kernel/mmio.rs`.
- Added public x86_64 kernel page-table walk diagnostics.
- Made kernel page faults fatal after one diagnostic.
- Refactored AHCI HBA register access to use explicitly mapped BAR5 virtual
  addresses.
- Added BAR5 logging/validation and PCI command readback verification.
- Added bounded AHCI controller/port initialization, port scan, IDENTIFY, read-only
  `sata0` block driver, and read bounds checks.
- Kept no-disk boots clean and preserved built-in QFS fallback behavior without faking a `root=sata0` policy.
