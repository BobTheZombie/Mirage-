# Mirage Boot Status

This page describes the current Mirage boot milestone: a minimal Limine-loaded
x86_64 kernel image that reaches the kernel idle loop under QEMU, records serial
diagnostics, and now runs a supervised boot skeleton using a compiled-in mock
manifest fixture. It is a bootable architecture skeleton, not a complete
operating system.

Related ownership and follow-on planning documents:

- [Memory ownership](MEMORY_OWNERSHIP.md)
- [Interrupt ownership](INTERRUPT_OWNERSHIP.md)
- [Boot module policy](BOOT_MODULE_POLICY.md)
- [Root filesystem path](ROOTFS_PATH.md)
- [Userspace loader plan](USERSPACE_LOADER_PLAN.md)

## What boots today

The current QEMU boot path is:

1. `make iso` builds the freestanding `mirage-kernel` ELF for the custom
   `x86_64-mirage` target.
2. The ISO packaging flow places the kernel ELF and Limine configuration into a
   BIOS/UEFI bootable image at `build/mirage.iso`.
3. QEMU boots the ISO using the same machine profile as the Makefile smoke path:
   `qemu-system-x86_64 -M q35 -m 256M -cdrom build/mirage.iso -display none
   -no-reboot` with serial output captured.
4. Limine transfers control to the Mirage kernel entry point.
5. The kernel initializes early x86_64 scaffolding, constructs the mechanism
   kernel state, runs the minimal supervisor bootstrap path, explicitly skips the
   QFS root mount and userspace init for this milestone, launches the supervised
   mock boot-service skeleton, checks a mock IPC echo path, and reaches the idle
   loop.

## QEMU baseline markers

The existing QEMU smoke-test baseline remains intentionally small. A boot is
considered to have reached the historical idle-loop proof when serial output
contains:

```text
Mirage kernel booting
Mirage reached idle loop
```

These markers prove that Limine entered the kernel, the kernel did not panic
before the idle loop, and QEMU can keep the skeleton image alive until the smoke
script timeout stops the virtual machine.


## Non-emulator x86_64 boot artifact baseline

`scripts/x86_64-boot-smoke.sh` is the local bootpath baseline that does not
launch an emulator. By default, `make smoke-x86_64-boot` runs `make kernel` and
then inspects `target/x86_64-mirage/release/mirage-kernel` with a
readelf-compatible tool. The script may also inspect a prebuilt artifact via
`KERNEL_ELF=/path/to/mirage-kernel scripts/x86_64-boot-smoke.sh`.

The baseline checks that the artifact is an ELF64 x86_64 image, that the ELF
entry address matches `_start`, and that the required low-level bootstrap and
linker symbols remain present: `_start`, `__mirage_x86_64_bootstrap`,
`__limine_requests_start`, `__limine_requests_end`, `__stack_top`,
`__bss_start`, and `__bss_end`. It also verifies that `.requests`,
`.requests_start_marker`, and `.requests_end_marker` are retained in the linked
artifact despite linker section garbage collection. This complements the QEMU
smoke test by proving the static Limine handoff shape before boot media or an
emulator is involved.

## Failure diagnostics

Early boot guards emit stable COM1 diagnostics before normal architecture
initialization when the boot handoff is incompatible. Known failure markers are:

```text
unsupported Limine base revision
```

This marker means Limine reported that Mirage's requested base revision is not
supported, so the kernel halts instead of continuing with unsafe assumptions
about the boot protocol contract.

## Supervised skeleton boot markers

The supervised boot skeleton adds serial diagnostics between the baseline boot
marker and the final idle-loop marker. The expected supervised skeleton markers
are:

```text
root mount attempt skipped: minimal boot milestone does not require QFS root yet
supervisor initialization succeeded: minimal registry entries=
userspace init attempt skipped: minimal boot milestone uses supervisor-only skeleton
loading boot manifest
boot manifest validated
launching service: echo-service
service running: echo-service
echo-service IPC check passed
Mirage reached idle loop
```

The `minimal registry entries=` line may include the current number of core
service registrations. Tests and documentation should match the stable prefix
rather than treating the count as an architectural ABI.

## What is real during QEMU boot

The current milestone has real, runnable pieces in the QEMU smoke path:

- A Rust `no_std` kernel binary linked for a freestanding x86_64 target.
- A Limine-based ISO build path that produces `build/mirage.iso`.
- Early serial logging through the COM1 path used by QEMU.
- Early x86_64 architecture setup, kernel-state construction, and the top-level
  boot flow that enters the idle loop.
- Kernel process and endpoint mechanisms sufficient for the minimal supervisor
  path to create a supervisor process and register core service endpoints.
- Minimal supervisor bootstrap state for service registry, recovery-manager
  placeholder state, driver-manager placeholder state, and capability-table
  initialization reporting.
- A supervised mock service launch admitted from a manifest-shaped fixture.
- A mock IPC request/response check against `echo-service` using kernel IPC and
  capability-facing supervisor code paths.
- A QEMU smoke script that builds the ISO, captures serial output, applies a
  timeout, and validates the baseline boot markers.
- A non-emulator x86_64 boot artifact smoke script that validates the linked
  kernel ELF before ISO packaging or QEMU execution.

## What is compiled-in or static fixture data

The supervised skeleton deliberately uses static inputs until Mirage has a real
boot-module and root-filesystem path:

- The boot manifest used in the non-`full-boot` smoke path is compiled into the
  kernel image rather than discovered from Limine modules or QFS.
- `echo-service` is a mock manifest service, not a separately loaded signed
  service binary.
- The mock service image identifier, module identifier, endpoint name, restart
  policy, and capability list are static fixture values.
- Manifest validation is structural fixture admission, not real cryptographic
  signature verification.
- The supervisor's recovery-manager and driver-manager state in the minimal boot
  path are initialized placeholders. They express ownership boundaries without
  claiming real crash recovery for hardware services.
- Core service registrations are fixed minimal skeleton registrations used to
  keep kernel mechanism separate from supervisor policy.

These fixtures are acceptable for the current proof because the goal is to prove
the supervised boot shape without faking hardware discovery, storage, QFS root
mounting, or signed module loading.

## What is unit/integration-test-only

Several Mirage subsystems already have useful code and tests but are not part of
the live QEMU boot path yet:

- QFS object formatting, lookup, journaling, and host-backed image operations are
  exercised through hosted tests and tools, not mounted as the live QEMU root.
- Supervisor crash/restart behavior for mock services is covered by tests and
  fixture-driven paths, not by real restarted driver processes during QEMU boot.
- Capability grant/check/revoke/transfer details have unit coverage and skeleton
  integration points, but the QEMU path only exercises a narrow mock IPC service
  capability scenario.
- POSIX, libc, and userspace-loader work remains ABI/planning/stub work and is
  not used to execute a GNU/POSIX program during QEMU boot.
- Storage, graphics, networking, and hardware-driver crates/docs may contain
  architectural plans, mock boundaries, or partial code, but they are not live
  hardware bring-up in the smoke path.

## What is stubbed or intentionally skipped

The current boot milestone intentionally avoids pretending the whole GNU/Mirage
architecture is complete.

- QFS root mounting is skipped in the minimal boot path.
- Userspace init is skipped in the minimal boot path.
- Supervisor service startup is limited to minimal core registrations plus the
  mock `echo-service` skeleton, not a complete signed service manifest.
- Hardware discovery and driver orchestration are not complete.
- POSIX/GNU compatibility is architectural scaffolding and ABI-oriented code, not
  a complete userspace environment.
- The libc surface is still a small set of stubs/shims and is not a full libc
  port.

## Explicit non-goals for this milestone

The following are explicitly **not** part of the current QEMU boot milestone:

- Real NVMe driver bring-up.
- Real AHCI driver bring-up.
- Real USB or xHCI storage bring-up.
- Real AMDGPU or native graphics bring-up.
- A full QFS root filesystem mounted during QEMU boot.
- A POSIX loader capable of launching GNU/POSIX programs.
- A complete libc port.
- Wayland compositor, Wayland protocol stack, or native desktop session startup.
- Networking driver/service bring-up.
- Real cryptographic signatures or production signed-module verification.

These are later milestones. They should be introduced only through explicit,
bounded work that preserves Mirage's small mechanism-focused kernel, privileged
supervisor policy layer, capability-secured IPC, restartable service model, and
non-Unix internals.

## Known blockers

Before Mirage can move beyond this supervised skeleton milestone, it needs:

- Interrupt and timer ownership robust enough for sustained kernel work beyond
  the current idle-loop proof. See [interrupt ownership](INTERRUPT_OWNERSHIP.md).
- A clearer memory-management and paging ownership model after Limine handoff.
  See [memory ownership](MEMORY_OWNERSHIP.md).
- A signed boot-module policy path that can validate and launch real supervisor
  services from boot media instead of compiled-in fixtures. See
  [boot module policy](BOOT_MODULE_POLICY.md).
- A real block-device path and root-filesystem mount path before QFS can serve as
  the native root. See [root filesystem path](ROOTFS_PATH.md).
- A userspace loader and process-start path before POSIX/GNU programs can run.
  See [userspace loader plan](USERSPACE_LOADER_PLAN.md).
- Expanded test coverage for boot, supervisor recovery, IPC/capability checks,
  and QFS object lookup.

## Next milestone

The next milestone is to replace fixture-only pieces of the supervised boot
skeleton with real boot inputs while preserving the QEMU baseline proof:

1. Keep the kernel mechanism-focused and avoid adding policy to the kernel.
2. Move the mock boot manifest toward Limine module discovery or QFS-backed
   manifest lookup.
3. Keep using mock restartable services until real signed service images exist.
4. Exercise broader IPC and capability checks between the kernel, supervisor,
   and mock services.
5. Preserve the QEMU smoke test as the baseline proof that the ISO still boots to
   the idle loop.

That next milestone should still avoid real NVMe, AHCI, USB, AMDGPU, full QFS
root, POSIX loader, libc port, Wayland, networking, and real cryptographic
signature work unless those pieces are introduced as explicitly bounded mocks,
test-only fixtures, or documentation-only plans.
