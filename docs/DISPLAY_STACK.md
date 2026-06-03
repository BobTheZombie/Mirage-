# Mirage Display Stack

Mirage native graphics are Wayland-only. X11 is not part of the base
architecture, and the native display path must not depend on Linux framebuffer,
Linux DRM/KMS userspace ABI, or X server assumptions.

The intended direction is:

```text
GPU driver service or signed module
    -> displayd
    -> Wayland compositor
    -> Wayland clients
```

## Layer responsibilities

### GPU driver service or module

The GPU layer owns hardware-facing mechanisms:

* device initialization after capability checks;
* MMIO access through granted mappings;
* DMA and GPU memory objects through granted memory authority;
* IRQ handling through granted interrupt authority;
* command queue or ring mechanics;
* display-output inventory surfaced to `displayd`;
* framebuffer or scanout buffer objects;
* crash/fault reporting to the supervisor.

It must not own compositor policy, session policy, X11 policy, or unrestricted
process access.

### `displayd`

`displayd` is the supervised display authority above GPU drivers. It should own:

* output registration;
* mode choice policy;
* scanout ownership;
* buffer-handle mediation;
* handoff to the compositor;
* recovery coordination when a GPU service crashes;
* display capability grants to trusted graphics components.

`displayd` communicates with GPU services through Mirage IPC and capabilities.

### Wayland compositor

The Wayland compositor is the native graphics server for user sessions. It owns:

* client surface policy;
* input/display composition policy;
* window management policy;
* presentation timing policy;
* protocol-level Wayland behavior.

The compositor should receive only the display and buffer capabilities it needs.

### Wayland clients

Native graphical applications are Wayland clients. POSIX/GNU programs may see
familiar userspace libraries, but the native Mirage graphics path remains
Wayland over Mirage services.

## X11 policy

X11 must not be part of the base architecture. Mirage should not require an X
server to boot a graphical session, run native graphical applications, or expose
GPU/display functionality.

XWayland may be added later as an optional compatibility layer for legacy X11
applications. If added, it should run above the Wayland compositor and must not
become a base dependency or a kernel/display-driver dependency.

## No Linux framebuffer API dependency

Mirage's framebuffer abstraction is an internal pixel-memory mechanism for boot,
mocking, and simple scanout buffers. Native graphics must not rely on Linux
`fbdev`, `/dev/fb*`, Linux framebuffer ioctls, or Linux kernel framebuffer
structures.

A Linux framebuffer compatibility shim, if ever implemented, belongs above the
native display stack and should translate to Mirage display services rather than
constraining driver design.

## No Linux DRM/KMS ABI dependency

Mirage may eventually provide compatibility for selected software, but native
GPU/display services should not expose Linux DRM/KMS as their foundational ABI.
The native objects are Mirage capabilities, IPC endpoints, memory handles,
framebuffer or scanout buffers, GPU resources, and Wayland-facing display
services.

## Capability requirements

Graphics components must receive only explicit authority:

| Component | Example capabilities |
| --- | --- |
| GPU service | PCI claim, MMIO mappings, DMA/IOMMU region, IRQ/MSI vectors, VRAM objects, firmware objects, service registration endpoint |
| `displayd` | GPU service endpoint, output-control authority, scanout-buffer authority, compositor handoff endpoint |
| Wayland compositor | display session authority, buffer allocation/import handles, input service endpoint, presentation endpoint |
| Wayland client | Wayland connection endpoint and client buffer handles granted by the compositor |

The supervisor remains responsible for policy, grants, revocation, restart, and
session ownership. The kernel enforces low-level capability validity and IPC
transport; it does not become a monolithic graphics stack.
