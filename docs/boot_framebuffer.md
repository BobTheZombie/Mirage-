# Mirage Boot Framebuffer

Limine can provide Mirage with a firmware-created linear framebuffer during
boot. This framebuffer is a simple pixel-addressable memory region already set
up by firmware and handed to the kernel through the bootloader protocol.

## Current use

Mirage currently uses the Limine-provided boot framebuffer only for early
boot-time diagnostics. The kernel writes directly into that memory so early
status information can appear on screen before the supervised graphics stack is
available.

This path is intentionally narrow. It is an early diagnostic fallback, not a
native graphics architecture.

## What this is not

Drawing into the boot framebuffer does not mean Mirage has any of the following:

* AMDGPU support;
* Intel GPU support;
* PCI GPU discovery;
* native GPU modesetting;
* monitor or EDID management;
* a production display driver;
* a display server;
* a Wayland compositor.

The boot framebuffer is only a firmware-created linear buffer that Mirage can
write to temporarily. It must not be treated as a substitute for the planned
capability-secured GPU and display-service model.

## Primary early diagnostics

Serial output remains the primary early diagnostic path. It is simpler, more
reliable during bring-up, easier to capture in emulators and on real hardware,
and less dependent on firmware graphics handoff details.

Framebuffer diagnostics are supplemental. They are useful when visible boot
status helps, but they should not replace serial logging as the authoritative
early debugging channel.

## Future direction

The intended graphics path remains service-oriented and capability-controlled.
Future work should move from the temporary boot framebuffer toward:

* a PCI GPU service that discovers and owns GPU devices under supervisor policy;
* write-combining framebuffer mappings for safer and faster scanout-buffer
  access;
* hardware mode setting through GPU-specific mechanisms rather than firmware
  leftovers;
* display handoff from early boot diagnostics to a supervised graphics service;
* an eventual Wayland compositor path through `displayd`.

In that model, the boot framebuffer is only an initial handoff object for early
visibility. Long-lived display ownership belongs to supervised graphics services,
with the supervisor granting capabilities and the kernel enforcing low-level
memory, IPC, and hardware authority.
