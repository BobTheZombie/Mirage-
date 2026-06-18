# Bare-metal boot diagnostics

Mirage now keeps boot diagnostics in three separate channels:

* **Boot milestone UI**: concise phase/status table. It owns the top-level boot progress view and must not receive raw hardware dumps.
* **Boot log ring**: fixed-size, no-heap ring (`BOOT_LOG_CAPACITY = 96`) containing concise breadcrumbs, phase transitions, faults, panic records, and overwrite counts.
* **Failure screen**: persistent framebuffer-owned fatal screen rendered after panic, fault, timeout, or explicit boot failure. It appends evidence and does not clear captured output.

Framebuffer evidence preservation is intentional. Once `Framebuffer [ONLINE]` is recorded, normal boot rendering suppresses blind clears by default (`DEFAULT_NO_FB_CLEAR_AFTER_BOOT = true`). Use only explicit clear APIs:

* `clear_for_boot_ui()` for the first boot UI paint before useful framebuffer evidence exists.
* `clear_for_mode_switch()` for a deliberate display mode transition.
* `clear_for_debug_shell()` when entering the debug shell.

Raw hardware dumps are disabled by default on framebuffer. Enable serial-oriented detail at build time with environment flags such as `MIRAGE_DEBUG_PCI=1`, `MIRAGE_DEBUG_RYZEN=1`, or `MIRAGE_DEBUG_RAW_HW_DUMP=1`. Runtime boot-argument parsing is not wired into this kernel path yet; compile-time constants in `src/kernel/boot_diagnostics.rs` document the current fallbacks for `mirage.boot.freeze_on_fail`, `mirage.debug.no_fb_clear_after_boot`, `mirage.debug.raw_hw_dump`, `mirage.debug.serial`, and `mirage.debug.fb_log`.

For the Dell Inspiron 15 5505 / Ryzen 5 4500U USB path, boot with serial attached if possible and watch the final `[ryzen NN]` or `[renoir NN]` breadcrumb. The suspected area is covered by CPU CPUID, topology, AMD SoC PCI inventory, AMDGPU detection, AMD xHCI detection, ACPI, storage, and userspace handoff breadcrumbs.

Status semantics remain honest: `Online` means operational, `Detected` means hardware was found, `Stub` means discovery-only or policy not implemented, and `Skipped` means absent or deliberately disabled.
