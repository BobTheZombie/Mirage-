# Mirage Memory Management

This document records Mirage's early memory-management contract for the x86_64
Limine boot path. It describes what is implemented now, what is intentionally
reserved for supervisor policy later, and which assumptions must remain true so
Mirage keeps a small mechanism-focused kernel rather than drifting into a
monolithic policy owner.

## 1. Limine memory map ownership after handoff

After Limine jumps to Mirage, the kernel treats the Limine handoff as immutable
boot evidence. Limine remains the source of initial topology, but ownership of
machine accounting transfers to Mirage immediately:

* the architecture bootstrap snapshots Limine responses into typed `BootInfo`;
* x86_64 early paging consumes that `BootInfo` before higher-level services run;
* the physical frame allocator ingests the boot memory map once and builds the
  live frame-ownership database from it;
* the supervisor may later make policy decisions, but it does not reinterpret the
  raw Limine memory map behind the kernel's back.

The boot memory map is therefore an input record, not a long-lived authority
source. Once the physical allocator is initialized, live allocation and
reservation state comes from Mirage's frame database.

## 2. Limine region classes and Mirage ownership

Mirage preserves the distinction between the original Limine region class and
the allocator's current ownership state. Only page-aligned, Limine-usable memory
is allocatable at bootstrap. Everything else is represented as reserved or
special-purpose until a future policy path explicitly changes that state.

| Limine kind | Mirage allocator kind | Bootstrap ownership rule |
| --- | --- | --- |
| `Usable` | `Usable` | Allocatable after page alignment, except for ranges later reserved for kernel boot artifacts, allocator metadata, page tables, the bootstrap stack, framebuffer, modules, or ACPI records. |
| `Reserved` | `Reserved` | Never allocated by the normal frame allocator. |
| `BootloaderReclaimable` | `BootloaderReclaimable` | Tracked separately from free memory; not automatically allocated during early boot. Future code may reclaim it only after all Limine-owned structures and references are no longer needed. |
| `KernelAndModules` | `Kernel` | Reserved as boot-owned executable/module memory. Mirage also explicitly reserves the kernel load range and boot modules, even if they overlap a broader map entry. |
| `Framebuffer` | `Mmio` | Reserved as device/MMIO-like memory, not normal RAM. Early display diagnostics may map it, but it is not frame-allocator free memory. |
| `AcpiReclaimable` / `AcpiNvs` | `Acpi` | Reserved for firmware/ACPI use. Mirage also reserves the RSDP page when Limine provides it. |
| `BadMemory` | `Reserved` | Never allocated. Treat as unusable physical address space. |
| unknown Limine values | `Reserved` | Fail closed: do not allocate memory with unknown firmware semantics. |

This conservative classification keeps the mechanism/policy split intact: the
kernel enforces that unsafe or externally-owned ranges do not enter the free
pool, while later reclamation policy belongs above the raw allocator path.

## 3. HHDM/direct-map assumptions and translation helpers

The current x86_64 path assumes Limine usually provides an HHDM (higher-half
direct map) offset. With an HHDM, Mirage keeps Limine's existing CR3-installed
address space, records the active PML4 physical address, and translates
page-table frames by adding the HHDM offset to physical addresses.

The active translation rules are:

* kernel image virtual addresses translate through the Limine executable-address
  response when available;
* other direct-mapped physical memory translates as `virtual = physical + hhdm`;
* direct-map virtual addresses translate as `physical = virtual - hhdm`;
* if no HHDM exists, early code falls back to identity-style translation and a
  static page-table pool for the limited bootstrap mapping path.

The public helpers that expose these assumptions are:

* `AddressTranslator::physical_for_virtual` and
  `AddressTranslator::virtual_for_physical` for early internal translation;
* `hhdm_virt_for_phys` and `hhdm_phys_for_virt` for checked HHDM conversions;
* `translate_virt` for walking active page tables and resolving a mapped virtual
  address to a physical address.

Frame-backed page-table growth currently requires the HHDM. If the HHDM is
missing, Mirage can perform only the static early mapping path and must not
pretend that the dynamic kernel heap/page-table path is available.

## 4. Physical frame allocator metadata placement and reservation

The physical allocator stores one `FrameState` byte per discovered 4 KiB frame.
During `ingest_boot_info` it:

1. copies Limine memory-map classes into Mirage physical regions;
2. reserves boot-owned ranges for the kernel image, boot modules, framebuffer,
   RSDP, static page-table pool, and a conservative bootstrap stack window;
3. computes the total frame count from the highest mapped physical end;
4. finds the first sufficiently large aligned `Usable` range for allocator
   metadata;
5. reserves that metadata range as `AllocatorMetadata` before building the live
   frame database;
6. initializes each frame as `Free`, `BootloaderReclaimable`, `Reserved`, or
   `Invalid` according to its region.

Allocator metadata is therefore self-hosted in real physical memory, but it is
removed from the free pool before any frame allocations can return it. This rule
also means allocator initialization can fail cleanly with `MetadataUnavailable`
or `MetadataTooSmall` rather than silently borrowing untracked memory.

## 5. x86_64 page-table mapping helpers and supported flags

The x86_64 paging layer currently maps 4 KiB pages through a PML4/PDPT/PD/PT
walk. It intentionally exposes small helpers instead of a broad VM policy API:

* `paging::initialize` captures or builds the early kernel mapping;
* `enable_frame_backed_mapping` allows new page-table pages to come from the
  physical allocator once it has ingested boot memory;
* `map_page` installs one page and validates alignment plus supported flag bits;
* `map_range` maps a page-aligned run by repeatedly calling `map_page`;
* `unmap_page` removes one mapping and returns the physical frame address;
* `map_kernel_page` translates `MemoryProtection` into kernel page flags;
* `create_user_address_space`, `switch_address_space`, and
  `destroy_user_address_space` are early address-space primitives for future
  userspace support.

Supported page flag bits are:

* present (`PRESENT`, automatically ensured by `map_page`);
* writable (`WRITABLE`);
* user accessible (`USER`);
* no execute (`NO_EXECUTE`);
* global (`GLOBAL`);
* write-through (`WRITE_THROUGH`);
* cache-disable (`CACHE_DISABLE`).

Flags outside this set fail with `UnsupportedFlags`. Page-table pages themselves
are mapped as present+writable kernel tables. The current implementation does
not yet expose huge-page mappings or PAT index selection through `PageFlags`.

## 6. Kernel heap bootstrap order

The intended bootstrap order is strict:

1. **Boot info.** The architecture bootstrap snapshots Limine state into
   `BootInfo`, validates the Limine base revision, and passes the typed handoff
   into x86_64 architecture initialization.
2. **Physical allocator.** `memory::initialize_from_boot_info` first calls the
   physical allocator's `ingest_boot_info`, so frame ownership is known before
   dynamic mappings consume frames.
3. **Mapper.** After the physical allocator is ready,
   `paging::enable_frame_backed_mapping` enables page-table growth backed by
   allocated physical frames. This step requires an HHDM.
4. **Heap reservation.** `heap::initialize` reserves the kernel heap virtual
   window at `0xffff_9000_0000_0000` with a 16 MiB virtual capacity.
5. **First committed heap pages.** The heap commits the first 128 KiB by
   allocating 4 KiB physical frames and mapping them read-write, global, and
   non-executable.
6. **Global allocator availability.** The Rust global allocator exists from the
   kernel image as a thin wrapper around Mirage's memory manager. Before
   promotion it can use the static bootstrap heap; after successful heap
   initialization the memory manager is promoted to the page-backed virtual heap.
   If any required step fails, the static heap is disabled instead of pretending
   the virtual heap exists.

Code that needs dynamic allocation during architecture bring-up must respect this
order. In particular, page-backed heap growth must not occur before both the
physical allocator and frame-backed mapper are online.

## 7. Future userspace address spaces

Mirage already has early hooks for userspace address spaces, but they are not yet
a complete POSIX process-memory implementation. A future address-space manager
should:

* keep kernel-half mappings shared while isolating user-half mappings;
* represent mappings as supervisor-granted memory capabilities, not ambient
  process rights;
* support explicit map, unmap, revoke, and crash cleanup paths;
* preserve syscall and IPC copy/check boundaries;
* support POSIX-compatible process behavior without making the kernel own
  high-level launch or service policy.

The kernel should remain the enforcement layer for page tables and capability
validity. The supervisor should decide which process or service receives which
memory object and when that authority is revoked.

## 8. Future DMA/IOMMU constraints and device-owned memory

Normal allocatable RAM, device MMIO, DMA buffers, and device-owned memory must be
kept distinct. Future DMA support should follow these constraints:

* DMA buffers are granted through explicit DMA/memory capabilities, never through
  unrestricted physical addresses;
* device-owned memory must be pinned, bounded, and attached to a driver service
  or kernel module lifetime;
* IOMMU domains should constrain each device to only the buffers and MMIO ranges
  granted by supervisor policy;
* driver restart must revoke capabilities, quiesce DMA, tear down IOMMU mappings,
  and only then recycle pages;
* cache-coherency and memory-type transitions must be recorded as part of the
  mapping contract, not hidden inside ordinary heap allocation.

This is especially important for supervised driver services: a crashed driver
must not keep DMA authority after the supervisor restarts it.

## 9. TODOs: huge pages and framebuffer PAT/write-combining

* Add 2 MiB and 1 GiB huge-page support for direct-map and large kernel ranges
  once the allocator can represent aligned multi-frame reservations cleanly.
* Teach the mapper to select PAT indices and document the cacheability contract
  for write-back, uncached, write-through, and write-combining mappings.
* Convert early framebuffer mappings to explicit write-combining mappings when a
  scoped PAT manager exists.
* Keep the boot framebuffer as an early diagnostic object only; long-lived GPU
  and scanout ownership must move to supervised graphics services.
