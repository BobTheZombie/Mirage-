# Mirage AMDGPU Early Hardware Status

## Current real support level

Mirage does not currently provide production-ready AMDGPU hardware support. The
default AMDGPU crate is a mock supervised GPU service. The `hw-amdgpu` feature
adds an early hardware-gated skeleton that validates AMD PCI identity, checks
supervisor-provided capabilities, wraps MMIO and VRAM BAR metadata, and exposes a
boot-framebuffer-like display model. It does not initialize AMD display engines,
load firmware, program modes, submit GPU commands, or handle interrupts.

Do not claim real AMDGPU support until Mirage initializes the relevant AMD GPU
blocks and performs validated display or command I/O on real hardware or a
validated emulator. Preserving or wrapping a boot framebuffer is not the same as
owning the GPU.

## What code is actually implemented

* Mock AMD PCI identity matching for selected ASIC families.
* Mock MMIO, VRAM, DMA, IRQ, firmware, display core, command ring, and
  framebuffer objects.
* Capability checks before mock device initialization and display operations.
* `hw-amdgpu` PCI identity extraction from a supervisor-provided PCI device.
* `hw-amdgpu` BAR layout metadata for MMIO and VRAM, checked MMIO read/write
  helper wrappers, VRAM aperture metadata, and boot framebuffer preservation
  structures.
* A hardware-shaped `AmdGpuDevice` that can expose framebuffer-style operations
  through the common GPU trait while still using software framebuffer state for
  display changes.

## What remains mocked

* Real PCI enumeration policy and BAR mapping setup are supplied by callers; the
  driver does not independently own platform discovery.
* AtomBIOS parsing, Display Core/DCE/DCN discovery, connector/encoder/CRTC
  programming, EDID parsing, and modesetting are not implemented.
* Firmware validation/loading, PSP/SMU interactions, command processor setup,
  ring buffers, doorbells, fences, scheduler integration, and acceleration are
  not implemented.
* VRAM management, GEM/TTM-like buffer objects, page tables, GPU virtual memory,
  and DMA command submission are not implemented.
* Interrupt handling, hotplug, reset recovery, power management, clock control,
  and suspend/resume are not implemented.
* The active framebuffer path is still a software/mock framebuffer abstraction,
  not a real DCN scanout programming path.

## Intended hardware or emulator target

The first target should be a validated emulator or controlled test machine where
firmware/bootloader has already provided a usable linear framebuffer for an AMD
GPU. Early validation should only prove PCI identity checks, capability checks,
BAR metadata handling, safe MMIO helper bounds, and preserved boot-framebuffer
writes. Real modesetting and acceleration require separate staged validation on
scratch hardware with a reliable recovery path.

## Known safety limitations

* AMDGPU register programming can wedge the GPU or display without reset support.
* Real firmware and command submission are absent; attempting to drive hardware
  beyond the skeleton would be unsafe.
* MMIO helper bounds do not prove that register sequences are correct or safe.
* VRAM metadata does not provide a real allocator, scanout ownership, or GPU
  memory-management isolation.
* There is no interrupt, hotplug, reset, power, or crash recovery path validated
  against real AMD hardware.
* This is not Linux DRM/KMS and must not expose Linux kernel internals as the
  native Mirage GPU ABI.

## TODO roadmap

1. Add a documented emulator or hardware-lab recipe for AMD PCI/BAR detection
   and boot-framebuffer preservation.
2. Define supervisor policy for GPU PCI, MMIO, VRAM, DMA, IRQ, firmware, reset,
   and power capabilities.
3. Add safe BAR mapping ownership and revocation through `mirage-hw`.
4. Parse AtomBIOS or equivalent firmware tables without importing Linux DRM as
   the native ABI.
5. Implement read-only display discovery: connectors, EDID, current mode, and
   boot scanout state.
6. Implement a minimal modeset path for one validated ASIC/display generation.
7. Add signed firmware loading, ring setup, fences, and interrupt delivery only
   after display discovery is stable.
8. Add supervisor crash cleanup: stop command submission, revoke DMA/MMIO/IRQ,
   reset the GPU when safe, and restart or quarantine the service.
9. Integrate with `displayd` and a Wayland compositor while keeping policy above
   the kernel.
