# Mirage Storage, AHCI, Block, Partition, and Boot Runtime Audit

Current working path inspected: `/workspace/Mirage-`.

## Current working path

- AHCI PCI discovery is driven from the platform registry and uses PCI class/subclass/prog-if to detect AHCI controllers.
- AHCI BAR5 is validated as MMIO, mapped through the kernel MMIO mapper, and read with volatile HBA register access.
- AHCI port scan reads `PxSSTS` and `PxSIG`, decodes DET/IPM, and classifies SATA, ATAPI, SEMB, port multiplier, empty, and unknown ports.
- SATA IDENTIFY uses a bounded command path with command list/FIS/command table/data frames allocated from the physical allocator and accessed through HHDM virtual aliases.
- SATA `read_blocks` issues ATA READ DMA EXT through a single bounce page and copies into the caller buffer.
- AHCI writes are policy-disabled by default; the current WRITE DMA EXT path returns read-only unless future mount-rw and kernel write-enable policy is wired.
- The generic kernel block layer now exposes static no-heap device registration, stable IDs, duplicate-name rejection, enumeration, lookup, and range validation.
- MBR and GPT parsers exist above `BlockDevice`, including MBR signature/range checks, protective MBR detection, GPT header CRC32, GPT entry-array CRC32, usable-LBA validation, and UTF-16LE name decoding.
- Boot Runtime RAMFS now has a simple immutable archive format, manifest parsing, hash verification via CRC32, path lookup, file read, and read-only enforcement.
- Spider-rs launch policy reads `/bootrt/sbin/spider-rs` from Boot Runtime and asks MTSS/kernel task creation to launch a userspace task instead of calling Spider-rs as a kernel function.

## Incomplete path

- AHCI command-slot management is still single-slot and must be generalized before queueing concurrent storage I/O.
- AHCI PRDT handling is single-entry and page-sized for the early boot path; large transfers must be split.
- ATAPI packet transport is honestly detected but not fully online: IDENTIFY PACKET, SCSI INQUIRY, READ CAPACITY(10), and READ(10) remain to be wired to `atapi0`.
- NVMe namespace registration is still separate future work.
- Partition block devices (`sata0p1`, `nvme0n1p1`) can be parsed but still need static wrapper-device registration over parent block devices in the boot path.
- QFS/ext4 root mount selection still depends on existing mount scaffolding and needs explicit device/partition handoff.
- Boot Runtime image production/staging in the ISO is format-defined but still needs the build artifact to be generated from the Spider-rs ELF and installed as a Limine module.
- Real ring-3 transition remains architecture loader work; the kernel now validates ELF bytes and creates an MTSS-visible task rather than falsely marking execution complete.

## Unsafe assumptions

- AHCI DMA memory is assumed reachable by the controller through physical frame addresses allocated by the early allocator.
- HHDM virtual aliases are used only for CPU access to DMA buffers; physical addresses are programmed into AHCI registers and PRDT entries.
- Cache coherency is assumed for QEMU/VirtualBox x86_64; future real hardware must add explicit cache/IOMMU policy.
- Boot Runtime hash verification is CRC32 only. Signature verification is not implemented and must not be logged as verified.

## Missing DMA pieces

- DMA-safe allocator metadata with alignment, physical address, virtual alias, size, and ownership tracking.
- Multiple command tables and PRDT entries per active port.
- Bounce-buffer pool for non-DMA-safe caller buffers and transfers larger than one page.
- IOMMU mappings and revocation for supervised driver services.
- Command timeout recovery that can reset a failed port without wedging the boot path.

## Exact plan to reach `sata0`/`atapi0` block devices

1. Keep AHCI controller Online when the controller initializes even if no SATA disk exists.
2. For each implemented port, log `PxSSTS`, decoded DET/IPM, `PxSIG`, and classification.
3. For SATA disks, stop the command engine with bounded waits, allocate CL/FIS/CT/data DMA frames, program `PxCLB/PxFB`, clear `PxSERR/PxIS`, start FRE/ST, issue IDENTIFY, parse geometry, and register `sata0`.
4. Implement multi-sector READ DMA EXT splitting through bounce buffers, preserving read-only default behavior for writes.
5. Implement WRITE DMA EXT and FLUSH CACHE EXT only behind explicit kernel write-enable and mount-rw policy.
6. For ATAPI devices, reuse the per-port DMA setup, issue IDENTIFY PACKET, SCSI INQUIRY, READ CAPACITY(10), and READ(10); register `atapi0` read-only only when media is present.
7. If ATAPI exists without media, mark ATAPI Detected/Online but Optical Disk Skipped with a no-media reason.

## Exact plan to mount Boot Runtime RAMFS

1. Stage a Boot Runtime image as a Limine module whose path or command line contains `bootrt`.
2. During kernel boot, find the module before normal rootfs mount policy.
3. Validate the Boot Runtime magic, manifest entries, file offsets/sizes, and per-file hash.
4. Mount it as immutable RAMFS at `/bootrt` and mark Boot Runtime Online only after `/bootrt/sbin/spider-rs` resolves.
5. If no image exists while Spider-rs is required, mark Boot Runtime Failed, not Skipped.

## Exact plan to launch Spider-rs from Boot Runtime

1. Mount Boot Runtime before normal rootfs selection.
2. Supervisor/userspace-loader path reads `/bootrt/sbin/spider-rs` from RAMFS.
3. The ELF validator parses the Spider-rs bytes and verifies a mapped entry point.
4. MTSS/kernel process creation creates PID 1 from the ELF entry.
5. Spider-rs is never invoked as a Rust function in kernel mode.
6. Spider-rs later mounts QFS/ext4 rootfs and starts normal services.
