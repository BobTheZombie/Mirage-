# Full Userspace Bring-up Audit

Date: 2026-06-12

## Scope

This audit inspected the current Mirage repository before code changes for the requested path:

```text
Limine -> seed-rs -> BootInfo -> kernel_main -> architecture bring-up -> platform discovery
-> Mirage-dispatch-rs -> drivers/services -> supervisor -> root FS -> MTSS
-> userspace ELF loader -> /sbin/spider-rs
```

## Executive status

Mirage is boot-architecture-skeleton complete, not full userspace-init complete. The tree already contains seed-rs, typed BootInfo construction, a no-heap boot phase manager, a platform registry, a kernel-internal dispatch framework, root filesystem probes, supervisor mock/service-manifest paths, MTSS run-queue primitives, ELF64 validation scaffolding, and Spider-rs host/no_std scaffolding.

The hard blocker for real `Spider-rs [ Online ]` remains the missing end-to-end ring-3 handoff: the loader cannot yet read `/sbin/spider-rs` bytes from the mounted root filesystem into a user address space, map PT_LOAD segments with real physical backing, install a task CR3, and enter user mode through `iretq`/sysret. The boot path must therefore report Userspace Loader and Spider-rs as Stub/Failed until that mechanism exists.

## Working

- **Core boot:** Limine entry and seed-rs handoff exist. `BootInfo` includes protocol, bootloader, memory map, framebuffer, serial, RSDP, HHDM, kernel image, and modules.
- **Architecture basics:** x86_64 serial, GDT, IDT/PIC/timer setup, framebuffer console initialization, and panic serial/framebuffer path are present.
- **Memory skeleton:** Boot memory map parsing, HHDM-dependent initialization, a physical allocator interface, paging initialization, and heap status reporting exist.
- **Boot state tracking:** `BootPhaseManager` is allocation-free and tracks registered phases with state/messages.
- **Platform registry:** `mirage-platform::PlatformRegistry` records discovered devices and now has authoritative query helpers for PCI class/id and common Mirage devices.
- **PCI discovery:** QEMU bus-0 CF8/CFC enumeration exists and now feeds one registry snapshot used by subsequent platform decisions.
- **Drivers (honest state):** framebuffer can reach Online when provided; I8042/PS2 can initialize when compiled; AHCI/NVMe/xHCI paths are detected or skipped/stubbed without blocking boot.
- **Supervisor:** creation, minimal bootstrap, full manifest bootstrap, mock echo service, capability checks, crash/restart tests, and root FS mount attempts exist.
- **MTSS:** portable run queue and kernel integration are present; kernel boot can mark MTSS Online for the scheduler skeleton.
- **Userspace validation:** ELF64 ET_EXEC/x86_64 validation and PT_LOAD bounds math exist.
- **Spider-rs:** host service-graph manager exists; a no_std PID1 ELF entry now exists for the future kernel loader path.

## Stub

- **AHCI/NVMe/xHCI drivers:** detection is present, but native controller probe/start is not implemented.
- **AMD/Ryzen platform services:** CPUID and PCI identification exist, but AMD IOMMU IVRS parsing, telemetry, and Renoir display driver bring-up are stubs/skips.
- **Userspace Loader:** validates ELF structure, but lacks root FS byte-read plumbing, real page allocation/mapping, and ring-3 entry.
- **Spider-rs kernel launch:** the PID1 ELF is buildable/staged, but kernel does not yet load it into ring 3; boot must not mark it Online.

## Broken

- **Workspace tests:** `cargo test --workspace --exclude mirage-boot` currently has existing failing kernel/supervisor tests unrelated to this pass (futex timeout, process/file/device/security regressions, and one nvmed recovery path).
- **Userspace load path:** `load_elf_from_file("/sbin/spider-rs")` intentionally returns `RootFsReadUnavailable`/NotFound semantics, preventing false Online status.

## Missing

- User CR3/address-space construction with copied kernel-safe mappings and no user-accessible kernel pages.
- Physical page allocation and copying for ELF PT_LOAD contents and BSS zeroing.
- User stack argument/auxv layout.
- x86_64 syscall entry/return setup for the Spider-rs ABI.
- Actual `iretq`/sysret transition into ring 3.
- Supervisor authorization object for PID 1 launch requests.
- QFS/rootfs install/read API usable by the no_std userspace loader.
- QEMU serial-log acceptance test that observes a real userspace write.

## Unsafe / high-risk

- PCI config-space probing uses raw I/O ports and should remain an architecture mechanism only.
- HHDM and paging setup are early-boot critical; user mappings must be added only after explicit user/supervisor permission bits are modeled.
- The syscall shim in no_std Spider-rs assumes the documented Mirage ABI and must not be executed until the kernel installs the matching syscall path.

## Duplicate

- Before this pass, platform code scanned PCI once to populate the registry and then scanned again through `pci_any` for AMD SoC, Renoir GPU, and AMD xHCI decisions. Those decisions now query the registry snapshot instead.

## Must-fix-before-Spider-rs Online

1. Root filesystem byte API for `/sbin/spider-rs`.
2. ELF64 loader that maps PT_LOAD code/data and zeros BSS into a user address space.
3. User stack layout.
4. Task/thread records bound to a user CR3/address space.
5. Ring-3 entry path and syscall return path.
6. Minimal syscalls: `write`, `getpid`, `yield`, `exit` with user-pointer validation.
7. Supervisor authorization record for PID 1 launch.
8. Mark `Spider-rs Online` only after the first userspace `write` succeeds.

## Build/QEMU state

- Existing `make qemu`/`make qemu-headless` build ISO paths remain.
- `make qemu-spider` now builds a no_std Spider-rs PID1 ELF, stages it at `build/rootfs/sbin/spider-rs`, includes it in the ISO tree when present, and invokes the QEMU boot path while reporting the kernel handoff honestly as Stub until the loader can consume the staged ELF.
