# QEMU Boot Guide

This guide covers building and running the Mirage x86_64 boot image under QEMU. The current QEMU path builds a Limine-bootable ISO at `build/mirage.iso`, then starts it with serial output on stdio.

## Dependencies

Install the host packages and tools needed for ISO creation and QEMU execution:

- `qemu-system-x86_64` — required to run the x86_64 virtual machine.
- `limine` artifacts — either provide a system Limine install or use the repo-managed download path. The default `make image` flow downloads Limine from `LIMINE_URL` into `build/limine`, builds it there, and uses artifacts such as `build/limine/limine`, `build/limine/limine-bios.sys`, `build/limine/limine-bios-cd.bin`, `build/limine/limine-uefi-cd.bin`, `build/limine/BOOTX64.EFI`, and `build/limine/BOOTIA32.EFI`.
- `xorriso` — required to generate the hybrid BIOS/UEFI ISO image.
- `mtools` — not required for the current ISO-only path, but install it if HDD or FAT image support is added.
- OVMF package — required for UEFI boot. Package names vary by distribution, commonly `ovmf`, `edk2-ovmf`, or similar. The QEMU scripts look for firmware at `/usr/share/OVMF/OVMF_CODE.fd`, `/usr/share/edk2-ovmf/x64/OVMF_CODE.fd`, or `/usr/share/qemu/OVMF.fd`; set `MIRAGE_OVMF_CODE=/path/to/OVMF_CODE.fd` if your firmware is elsewhere.

The build scripts also require the Rust toolchain, `cargo`, `rustup`, `curl`, `tar`, and `make`.

## Build

Build the QEMU ISO directly with:

```sh
tools/build-qemu-image.sh
```

Or use the Makefile target:

```sh
make image
```

Both paths produce the default ISO at:

```text
build/mirage.iso
```

The script defaults to the full QEMU feature set (`hw-framebuffer full-boot`) and retries a minimal framebuffer build (`hw-framebuffer`) if the full boot build fails. You can override feature selection with `QEMU_FEATURES`, `MIRAGE_QEMU_FEATURES`, or related Makefile variables.

## Graphical framebuffer run

Run Mirage in graphical QEMU with:

```sh
tools/run-qemu.sh
```

Or use the Makefile target:

```sh
make qemu
```

The graphical runner builds the ISO unless `MIRAGE_SKIP_BUILD=1` is set, then launches `qemu-system-x86_64` with the ISO as `-cdrom`, serial output on stdio, and Q35 machine defaults. When `/dev/kvm` is accessible, it uses KVM and `-cpu host`; otherwise it falls back to emulated CPU settings.

## Headless serial run

Run without a display and keep serial output attached to the terminal:

```sh
tools/run-qemu-headless.sh
```

Or use the Makefile target:

```sh
make qemu-headless
```

This wrapper delegates to `tools/run-qemu.sh` with `-display none`, so the serial console remains the primary way to observe boot progress.

## Debug

Start QEMU paused with the built-in GDB stub enabled:

```sh
tools/run-qemu-debug.sh
```

Or use the Makefile target:

```sh
make qemu-debug
```

Then attach GDB from another terminal:

```gdb
target remote :1234
```

The debug wrapper passes `-S -s` to QEMU, which waits for the debugger before executing guest code.

## Expected serial output

A successful early boot should print the following serial milestones in order:

```text
Mirage kernel booting...
serial initialized
```

Then one of the framebuffer status lines:

```text
Mirage framebuffer online
```

or:

```text
framebuffer unavailable; serial console only
```

Finally, the kernel should reach the idle loop:

```text
Mirage reached idle loop
```

Additional diagnostic lines, such as framebuffer resolution, pitch, bits per pixel, or address, may appear when framebuffer initialization succeeds.

## Expected framebuffer behavior

Graphical QEMU should display early framebuffer diagnostics once the `hw-framebuffer` feature is active and Limine provides framebuffer metadata to the kernel. Serial output remains authoritative, but the framebuffer console mirrors early diagnostics on a best-effort basis when a validated framebuffer is available.

If the kernel prints `framebuffer unavailable; serial console only`, the boot may still be healthy; it means the current boot path did not expose usable framebuffer metadata or the `hw-framebuffer` path was not active.

## Troubleshooting

### Missing OVMF

Symptoms:

- The runner reports that the selected image requires UEFI but no OVMF firmware was found.
- UEFI boot fails before Mirage starts.

Fixes:

- Install your distribution's OVMF package, commonly `ovmf` or `edk2-ovmf`.
- Set `MIRAGE_OVMF_CODE=/path/to/OVMF_CODE.fd` if the firmware is not in one of the default search paths.
- Force BIOS boot with `MIRAGE_QEMU_FIRMWARE=bios` only when the image has Limine BIOS support.

### Missing Limine binary or artifacts

Symptoms:

- `make image` or `tools/build-qemu-image.sh` fails while copying or checking `build/limine/*` files.
- `build/limine/limine` or Limine BIOS/UEFI files are absent.

Fixes:

- Run `make limine` to download and build repo-managed Limine artifacts.
- Remove stale artifacts with `rm -rf build/limine build/limine-binary.tar.xz`, then run `make image` again.
- Check network access if the Limine release download fails.

### Missing `xorriso`

Symptoms:

- The build fails with `missing required command 'xorriso'` or at the ISO creation step.

Fixes:

- Install `xorriso` using your host package manager.
- Re-run `tools/build-qemu-image.sh` or `make image`.

### No framebuffer

Symptoms:

- Serial output says `framebuffer unavailable; serial console only`.
- Graphical QEMU opens, but no Mirage framebuffer diagnostics appear.

Fixes:

- Build with `hw-framebuffer` enabled, for example `QEMU_FEATURES=hw-framebuffer make image` or the default QEMU build path.
- Use the graphical runner rather than the headless runner.
- Confirm Limine is booting the kernel and providing framebuffer metadata.
- Continue using the serial console for diagnostics; lack of framebuffer does not necessarily indicate a failed boot.

### Triple fault or reboot loop

Symptoms:

- QEMU repeatedly resets.
- The VM exits or loops before expected serial milestones.

Fixes:

- Use `make qemu-debug` and attach with `gdb`, then inspect the first faulting instruction.
- Keep `-no-reboot -no-shutdown` enabled through the provided scripts so the failure state remains visible.
- Verify that the image was rebuilt after kernel or linker changes.
- Check for early boot regressions in architecture entry, page-table setup, interrupt setup, or Limine handoff handling.

### No serial output

Symptoms:

- QEMU starts but the terminal never prints `Mirage kernel booting...`.

Fixes:

- Use the provided scripts, which pass `-serial stdio`.
- Avoid adding QEMU flags that redirect or disable the serial device.
- Verify that the ISO exists at `build/mirage.iso` or set `MIRAGE_ISO_IMAGE` to the intended image.
- Try `tools/run-qemu-headless.sh` to remove graphical display variables from the test.
- Rebuild the image to ensure the latest kernel is included.

### Host without KVM

Symptoms:

- `/dev/kvm` is missing or inaccessible.
- QEMU runs slowly or the script does not add `-enable-kvm`.

Fixes:

- This is supported: the runner falls back to emulation with `-cpu max` when available, otherwise `-cpu qemu64`.
- Install and enable host virtualization support if you need faster boot testing.
- Check user permissions for `/dev/kvm` if KVM exists but is not readable and writable by your user.
