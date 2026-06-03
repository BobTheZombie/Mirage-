# Mirage AMDGPU Scaffold

The Mirage AMDGPU crate is an explicit mock scaffold for proving GPU, display,
framebuffer, IPC, and capability boundaries. It is not a production AMDGPU
driver, not a Linux DRM port, and not suitable for real hardware control.

## Current scope

The scaffold may model:

* minimal AMD PCI identity matching;
* mock ASIC family classification;
* a fixed MMIO aperture description;
* a fixed DMA region identifier;
* a fixed IRQ line;
* a fixed VRAM aperture and VRAM object;
* placeholder firmware metadata;
* a mock graphics command ring;
* a mock display core with fixed modes;
* a linear framebuffer backed by mock memory;
* capability checks before device initialization.

This scope exists so Mirage can test the architecture boundary: a GPU-like
driver receives narrowly scoped authority, exposes display objects to `displayd`,
and avoids hardcoding Linux driver assumptions into the kernel.

## Explicit non-production status

The AMDGPU scaffold does not implement production GPU support. In particular, it
does not provide:

* real PCI enumeration;
* real BAR probing or mapping;
* real AtomBIOS parsing;
* signed firmware loading;
* microcode upload;
* command processor initialization;
* real ring doorbells, write pointers, fences, or scheduler integration;
* VRAM memory management;
* GEM/TTM/DRM buffer management;
* real modesetting;
* EDID parsing;
* Display Core Next (DCN) or Display Core Engine (DCE) programming;
* interrupt handling;
* power management;
* reset recovery;
* acceleration for production clients.

Any documentation, API, or test using this crate must preserve that status. The
scaffold is allowed to be useful for demos and unit tests, but it must not imply
that Mirage can safely drive AMD hardware.

## Capability requirements

A future real AMDGPU service or signed module would need supervisor-granted
capabilities before initialization:

| Resource | Required capability |
| --- | --- |
| PCI function | Claim authority for the specific AMD GPU PCI device |
| MMIO BARs | Mapping authority for the specific register apertures |
| DMA | Bounded DMA region or IOMMU domain authority |
| IRQ/MSI/MSI-X | Interrupt routing authority for the device vectors |
| VRAM aperture/object | Memory-object authority for framebuffer and GPU buffers |
| Firmware modules | Access to signed firmware objects approved by supervisor policy |
| IPC endpoints | Registration authority for GPU/display service endpoints |
| Reset/power operations | Explicit device-control authority |

These capabilities must be revocable. If the service crashes, the supervisor
must be able to revoke hardware access, reclaim resources, and restart or keep
the GPU service offline without panicking the kernel.

## Driver placement

AMDGPU should normally be a supervised driver service because the display stack
benefits from crash isolation and restartability. A signed loadable module may be
considered only for kernel-adjacent mechanisms that cannot safely live in a
service. Such a module still must not own display policy, compositor policy, or
unrestricted hardware access.

## No Linux DRM or framebuffer dependency

The Mirage AMDGPU direction must not depend on Linux DRM/KMS, Linux `fbdev`, or
Linux kernel internal APIs. Mirage can learn from existing hardware documents and
open-source drivers, but native interfaces should remain Mirage IPC,
capabilities, framebuffer objects, GPU memory handles, and display service
messages.

A future POSIX/Linux compatibility shim may translate selected userspace
expectations, but that shim is not the native Mirage GPU ABI.
