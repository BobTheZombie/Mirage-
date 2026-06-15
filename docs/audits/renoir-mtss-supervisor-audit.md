# Audit: Mirage Renoir / MTSS / Supervisor patch set

## Existing repo structure observed

- `crates/mirage-ryzen` already owns Ryzen/AMD CPU platform facts.
- `crates/mirage-amd-chipset` already owns AMD chipset/PCI candidate metadata.
- `crates/mirage-mtss` exists as the multitasking subsystem crate.
- `src/supervisor` already contains the Supervisor/MTSS policy boundary.
- `src/arch/x86_64` owns the low-level boot path and is the right place for early Renoir-safe probing.

## Architectural issue fixed

The previous SoC candidate layer was not enough for proper boot.  A Ryzen 4500U needs lower-kernel support for early CPU/platform classification before supervisor driver policy can mean anything.

This patch therefore adds:

1. Lower-kernel Renoir detection under `src/arch/x86_64/platform/amd/`.
2. MTSS scheduler modules under `crates/mirage-mtss`.
3. Supervisor authorization under `src/supervisor/renoir_mtss.rs`.
4. Ryzen/chipset helper descriptors under `crates/mirage-ryzen` and `crates/mirage-amd-chipset`.

## Safety model

The patch is intentionally discovery-only for risky devices.  It reports AMDGPU, xHCI, IOMMU, and PSP candidates but does not take destructive or irreversible actions.
