# Bare-metal boot diagnostics audit

## Findings fixed

* The persistent boot screen previously cleared the framebuffer on every render. It now routes through `clear_for_boot_ui()`, which suppresses clears after framebuffer online by default.
* The generic early console no longer mirrors arbitrary `kprintln!` text to framebuffer after framebuffer online unless the framebuffer log overlay fallback is enabled. Serial remains authoritative.
* Panic handling now records a no-heap panic breadcrumb and draws the persistent failure screen before halt.
* Boot phase transitions now mirror into the boot diagnostics record and ring buffer.
* Ryzen/Renoir discovery now records explicit breadcrumbs for CPUID leaves, family/model parsing, brand/topology handling, MSR telemetry policy, AMD SoC PCI inventory, AMDGPU detection, AMD xHCI detection, and platform inventory exit.
* Raw PCI detail is gated by compile-time debug flags and remains serial-first; framebuffer gets concise phase/UI data by default.

## Known limitations

* Runtime boot-argument parsing for `mirage.*` flags is not yet available in this early path. Compile-time constants and `MIRAGE_DEBUG_*` environment options are documented fallbacks.
* Fault handlers can call `boot_trace_fault`, but full trap-vector integration depends on architecture IDT call sites wiring register values into the diagnostics API.
* Debug-shell commands are documented and backed by stored diagnostics, but the interactive parser may still be milestone-limited.

## Real hardware procedure

1. Build a USB image with serial enabled and, if needed, `MIRAGE_DEBUG_PCI=1` or `MIRAGE_DEBUG_RYZEN=1`.
2. Boot the Dell Inspiron 15 5505.
3. Do not reset immediately if the display turns solid blue or freezes; photograph the preserved framebuffer.
4. Capture serial output and locate the last `[ryzen NN]`, `[renoir NN]`, `[acpi NN]`, `[ahci NN]`, `[nvme NN]`, `[xhci NN]`, or userspace breadcrumb.
5. Treat `Detected` and `Stub` statuses as non-operational; only `Online` indicates a working device path.
