# Mirage Framebuffer Abstraction

The Mirage framebuffer abstraction is a small, generic linear framebuffer model
for boot graphics, display-service mocks, and GPU-driver scaffolds. It is not the
Linux framebuffer API and must not depend on `/dev/fb*`, Linux `fbdev` ioctls, or
Linux kernel display structures.

## Responsibilities

The framebuffer layer owns only low-level pixel-buffer mechanics:

* validated framebuffer modes;
* width and height;
* pitch/stride in bytes;
* bits per pixel;
* supported pixel format;
* framebuffer byte length validation;
* pixel offset calculation;
* safe mock memory for tests and scaffolds;
* simple pixel writes and clears.

Higher-level display policy belongs elsewhere. The framebuffer layer does not
choose monitors, perform compositor scheduling, allocate client surfaces, parse
EDID, own KMS policy, or expose a POSIX/Linux ABI.

## Supported pixel formats

The initial supported formats are deliberately small:

| Format | Memory byte order | Supported depths | Notes |
| --- | --- | --- | --- |
| `Rgb` | red, green, blue | 24 bpp or 32 bpp | At 32 bpp, the extra byte is padding/reserved by the mode contract. |
| `Bgr` | blue, green, red | 24 bpp or 32 bpp | Useful for common bootloader-provided linear modes. |
| `Xrgb` | unused, red, green, blue | 32 bpp | The unused byte is written as zero by the abstraction. |

All modes must have nonzero dimensions, byte-aligned pixel depth, and a pitch at
least as large as `width * bytes_per_pixel`.

## No Linux framebuffer dependency

Mirage may provide a POSIX-compatible userspace surface, but native Mirage
display services must not be built on Linux `fbdev`. The framebuffer abstraction
is an internal Mirage mechanism that can be backed by:

* bootloader-provided linear framebuffer memory;
* a mock memory vector in tests;
* a GPU service's capability-protected VRAM object;
* a future display service buffer allocation.

If a compatibility layer eventually emulates Linux framebuffer behavior for a
specific program, that emulation should live above Mirage display services and
must not become the native driver ABI.

## Capability requirements

A service drawing into a framebuffer needs authority to the memory object or
VRAM object backing that framebuffer. A GPU/display service may also require
capabilities for MMIO, DMA, IRQs, and display outputs, but the framebuffer object
itself should remain a narrow pixel-memory handle.

## Place in the display stack

The framebuffer abstraction is useful for early boot and scaffolding, but it is
not Mirage's final native desktop model. Native graphics direction is:

```text
GPU service or module
    -> displayd
    -> Wayland compositor
    -> Wayland clients
```

The framebuffer can be one transport or fallback object inside that stack; it is
not a substitute for the Wayland-native direction.
