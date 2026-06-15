# Mirage Renoir / Ryzen 5 4500U Kernel + MTSS + Supervisor Bring-up

This patch set adds a proper lower-kernel Renoir boot profile and connects it to
MTSS scheduler-module selection and supervisor authorization.

## What this patch does

- Adds lower-kernel x86_64 AMD Renoir boot profile detection.
- Detects AMD family `0x17`, model `0x60..0x7f` as Renoir/Lucienne-class Zen 2 mobile.
- Treats 6-core / 6-thread Renoir as Ryzen 5 4500U-class.
- Marks AMD SoC, Ryzen CPU, Ryzen topology, AMD IOMMU, AMDGPU Renoir, and AMD xHCI with honest boot states.
- Adds an MTSS scheduler-module registry with generic and AMD Zen 2 Renoir modules.
- Adds supervisor policy approval for architecture-specific MTSS modules.
- Adds config symbols and cargo features for the path.

## What this patch intentionally does not do

- Does not reset or modeset the GPU.
- Does not enable IOMMU translation.
- Does not start xHCI or USB HID.
- Does not bind AHCI/NVMe/AMDGPU driver services.
- Does not mark device drivers Online unless the existing real driver path does it.

## Expected boot semantics

A real 4500U-class machine should be able to report:

```text
Ryzen CPU       Ok / Detected
Ryzen Topology  Ok
AMD SoC         Ok
AMD IOMMU       Stub: discovery only
AMDGPU Renoir   Stub: discovery only
AMD xHCI        Stub: discovery only
MTSS            Online
Supervisor      Ok
```

MTSS should select:

```text
mtss-sched-amd-zen2-renoir
```

Supervisor policy must approve that module before it is treated as active.
