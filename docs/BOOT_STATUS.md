# Mirage Boot Status

This page describes the current Mirage boot milestone: a minimal Limine-loaded
x86_64 kernel image that reaches the kernel idle loop under QEMU and records
serial diagnostics. It is a bootable architecture skeleton, not a complete
operating system.

## What boots today

The current boot path is:

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
   QFS root mount and userspace init for this milestone, and reaches the idle
   loop.

The smoke-test success markers are:

```text
Mirage kernel booting
Mirage reached idle loop
```

## What is real

The current milestone has real, runnable pieces:

- A Rust `no_std` kernel binary linked for a freestanding x86_64 target.
- A Limine-based ISO build path that produces `build/mirage.iso`.
- Early serial logging through the COM1 path used by QEMU.
- The top-level kernel boot flow that creates kernel state and enters the idle
  loop.
- Minimal supervisor bootstrap registration used to prove the mechanism/policy
  layering without claiming a full service stack.
- A QEMU smoke script that builds the ISO, captures serial output, applies a
  timeout, and validates the boot markers.

## What is stubbed or intentionally skipped

The current boot milestone intentionally avoids pretending the whole GNU/Mirage
architecture is complete.

- QFS root mounting is skipped in the minimal boot path.
- Userspace init is skipped in the minimal boot path.
- Supervisor service startup is limited to a minimal bootstrap demonstration, not
  a complete signed service manifest.
- Hardware discovery and driver orchestration are not complete.
- POSIX/GNU compatibility is architectural scaffolding and ABI-oriented code, not
  a complete userspace environment.
- The libc surface is still a small set of stubs/shims and is not a full libc.

The following are explicitly **not** part of this boot milestone:

- NVMe driver bring-up.
- AHCI driver bring-up.
- USB or xHCI storage bring-up.
- AMDGPU or native graphics bring-up.
- Full QFS implementation as a mounted root filesystem.
- POSIX layer completion.
- libc expansion beyond the current stubs/shims.

## Known blockers

Before Mirage can move beyond this minimal boot milestone, it needs:

- Interrupt and timer ownership robust enough for sustained kernel work beyond
  the current idle-loop proof.
- A clearer memory-management and paging ownership model after Limine handoff.
- A signed boot-module policy path that can validate and launch real supervisor
  services.
- A real block-device path and root-filesystem mount path before QFS can serve as
  the native root.
- A userspace loader and process-start path before POSIX/GNU programs can run.
- Expanded test coverage for boot, supervisor recovery, IPC/capability checks,
  and QFS object lookup.

## Next milestone

The next milestone is to turn the idle-loop proof into a supervised boot
skeleton:

1. Keep the kernel mechanism-focused and avoid adding policy to the kernel.
2. Add a small signed boot-module manifest fixture.
3. Have the supervisor validate the fixture and launch a mock restartable service.
4. Exercise IPC and capability checks between the kernel, supervisor, and mock
   service.
5. Preserve the QEMU smoke test as the baseline proof that the ISO still boots to
   the idle loop.

That next milestone should still avoid NVMe, AHCI, USB, AMDGPU, full QFS root,
full POSIX, and expanded libc work unless those pieces are introduced as
explicitly bounded mocks or documentation-only plans.
