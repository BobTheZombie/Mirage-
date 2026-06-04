# Mirage Framebuffer Hardware Status

## Current real support level

Mirage does not currently provide production-ready framebuffer hardware support.
The default framebuffer is an in-memory mock used for tests, demos, and driver
scaffolds. The `hw-framebuffer` feature adds wrappers for caller-provided
framebuffer memory and checked pixel writes, but it does not discover bootloader
framebuffers by itself, program display hardware, parse EDID, or manage modes.

Do not claim real display support solely because pixels can be written into a
provided memory range. A real support claim requires a validated firmware,
emulator, or hardware framebuffer handoff and verified visible output under the
stated target.

## What code is actually implemented

* Pixel format and framebuffer mode validation for small RGB/BGR/XRGB-style
  formats.
* Checked framebuffer size and pixel offset calculations.
* An in-memory `MockFramebuffer` backend with pixel, rectangle, clear, and buffer
  operations.
* A `FramebufferTarget` trait and simple text-console facade that paints coarse
  cells rather than rendering a production font stack.
* With `hw-framebuffer`, a physical-framebuffer descriptor, a caller-provided
  mapped-memory wrapper, and a `FramebufferWriter` for checked writes into that
  memory.

## What remains mocked

* Firmware or bootloader framebuffer discovery and validation.
* Physical-to-virtual mapping setup, cache attribute selection, and lifetime
  ownership of real framebuffer memory.
* Modesetting, monitor selection, EDID parsing, page flipping, vsync, damage
  tracking, multi-output support, and GPU buffer management.
* Fonts, text shaping, terminal semantics, panic-console integration policy, and
  Wayland/display-service integration.
* Recovery after display memory is revoked or a display service crashes.

## Intended hardware or emulator target

The first target should be a bootloader- or emulator-provided linear framebuffer
with known dimensions, pitch, pixel format, and mapped address supplied to the
framebuffer crate by trusted boot/display setup code. QEMU with a simple linear
framebuffer handoff is the preferred initial validation environment. Native GPU
modesetting belongs in device-specific services such as AMDGPU plus `displayd`,
not in this crate.

## Known safety limitations

* The hardware writer trusts the caller-provided memory mapping and length.
* Incorrect framebuffer metadata can write outside the intended display buffer if
  mapping setup lies or aliases unrelated memory.
* No synchronization exists for concurrent writers, display scanout, or service
  revocation.
* Cacheability and memory-ordering rules are not fully specified for real
  hardware.
* This crate is not a Linux `fbdev` implementation and should not become the
  native display ABI.

## TODO roadmap

1. Add a documented QEMU/bootloader framebuffer validation recipe.
2. Define a supervisor-owned framebuffer memory capability and revocation model.
3. Validate physical memory ranges, pitch, size, and pixel format at handoff.
4. Add cache attribute guidance for framebuffer mappings.
5. Add a panic/early-console integration point that remains policy-light.
6. Add handoff from early framebuffer to `displayd` without making the kernel own
   display policy.
7. Add tests for mapped framebuffer bounds and visible-output smoke tests in a
   validated emulator.
