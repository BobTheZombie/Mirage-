# Mirage Configuration Utility

`mirageconfig` is the host-side configuration utility for GNU/Mirage kernel builds. It keeps build policy outside the kernel: the tool reads a declarative schema, validates a user configuration, and emits small build artifacts consumed by `make`, Cargo, and QEMU scripts.

## Source of truth

The schema lives at:

```text
config/MirageConfig.toml
```

Each `[[options]]` entry defines:

- `symbol` — Linux-style `CONFIG_...` symbol name.
- `prompt` — menu/list display text.
- `category` — one of the Mirage configuration categories such as Architecture, Boot, Hardware, Kernel Core, Memory, or Debug.
- `default` — default value used by `make defconfig` and missing-option migration.
- `help` — human-readable explanation.
- `depends_on` — symbols that must be enabled before this symbol may be enabled.
- `selects` — symbols automatically enabled when this symbol is enabled.
- `visible_if` — menu visibility predicates.
- `type` — currently `bool`; the internal representation is prepared for future `tristate`, `string`, `int`, and `hex` values.
- `cargo_feature` — optional mapping from a configuration symbol to an existing Cargo feature.

The menu and generator load this schema at runtime. Configuration symbols are not hardcoded into the menu UI.

## User configuration: `mirage.conf`

The primary user config file is:

```text
mirage.conf
```

It uses Linux-style assignment syntax:

```text
CONFIG_MIRAGE_HW_FRAMEBUFFER=y
CONFIG_MIRAGE_HW_PCI=n
```

For bool values, `y` means enabled and `n` means disabled. Existing comments are preserved where practical when the tool rewrites a config.

If `mirage.conf` is missing, build-oriented Make targets automatically run `make defconfig` first and continue with a default configuration.

## CLI usage

The host tool lives in:

```text
tools/mirageconfig/
```

It can be invoked directly through Cargo:

```sh
cargo run -q -p mirageconfig -- --list
cargo run -q -p mirageconfig -- --defconfig
cargo run -q -p mirageconfig -- --oldconfig
cargo run -q -p mirageconfig -- --check
```

Supported options:

- `--menu` — interactive bool menu. In non-interactive terminals, it falls back to oldconfig-style defaults.
- `--defconfig` — write a default `mirage.conf` from schema defaults and `selects`.
- `--oldconfig` — merge an existing config with schema defaults, apply `selects`, and rewrite it.
- `--savedefconfig` — write only values that differ from defaults. The default output is `mirage.defconfig` unless `--output` is supplied.
- `--list` — print all schema options grouped by category.
- `--check` — validate syntax and semantics.
- `--config <file>` — choose the input/output config file.
- `--output <file>` — choose the output file for write commands.

The Makefile also passes `--generate` for build integration, which emits generated artifacts under `target/mirage/config/`.

## Validation and dependency handling

`mirageconfig` validates:

- malformed schema syntax in `config/MirageConfig.toml`;
- duplicate or unknown symbols;
- malformed `CONFIG_SYMBOL=value` lines;
- invalid values for the option type;
- missing required options during strict checks;
- unmet `depends_on` relationships;
- circular `selects` graphs.

Select relationships automatically enable implied symbols. Examples in the schema include:

- `CONFIG_MIRAGE_HW_RYZEN` selects `CONFIG_MIRAGE_HW_AMD64`.
- `CONFIG_MIRAGE_HW_AMD_CHIPSET` depends on `CONFIG_MIRAGE_HW_RYZEN` and selects `CONFIG_MIRAGE_HW_PCI`.
- `CONFIG_MIRAGE_HW_NVME`, `CONFIG_MIRAGE_HW_AHCI`, and `CONFIG_MIRAGE_HW_XHCI` depend on `CONFIG_MIRAGE_HW_PCI`.

Dependencies are intentionally explicit so Mirage remains capability- and service-oriented rather than accidentally growing monolithic hardware assumptions.

## Generated artifacts

Build artifacts are generated under:

```text
target/mirage/config/
```

Generated files:

- `generated.rs` — no-std-compatible Rust constants such as:

  ```rust
  pub const CONFIG_MIRAGE_HW_FRAMEBUFFER: bool = true;
  ```

  The file contains only constants and does not require heap allocation or `std` in kernel-side code.

- `cargo_features.env` — shell environment file with derived Cargo features:

  ```sh
  MIRAGE_FEATURES="full-boot hw-framebuffer"
  ```

- `build_flags.env` — shell environment file with derived QEMU/build metadata:

  ```sh
  MIRAGE_QEMU_GRAPHICAL=1
  MIRAGE_QEMU_SERIAL_ARGS="-serial stdio"
  MIRAGE_QEMU_DISPLAY_ARGS=""
  MIRAGE_QEMU_DEBUG_ARGS=""
  MIRAGE_KERNEL_CMDLINE="mirage.verbose=1"
  ```

## Cargo feature generation

The root `Cargo.toml` preserves the existing Mirage feature names. The schema maps configuration symbols to those feature names:

| Symbol | Cargo feature |
| --- | --- |
| `CONFIG_MIRAGE_HW_FRAMEBUFFER` | `hw-framebuffer` |
| `CONFIG_MIRAGE_FULL_BOOT` | `full-boot` |
| `CONFIG_MIRAGE_HW_PCI` | `hw-pci` |
| `CONFIG_MIRAGE_HW_AMD64` | `hw-amd64` |
| `CONFIG_MIRAGE_HW_RYZEN` | `hw-ryzen` |
| `CONFIG_MIRAGE_HW_AMD_CHIPSET` | `hw-amd-chipset` |
| `CONFIG_MIRAGE_HW_AMD_IOMMU` | `hw-amd-iommu` |
| `CONFIG_MIRAGE_HW_NVME` | `hw-nvme` |
| `CONFIG_MIRAGE_HW_AHCI` | `hw-ahci` |
| `CONFIG_MIRAGE_HW_XHCI` | `hw-xhci` |

If `MIRAGE_FEATURES` is set in the environment, Make uses it instead of `mirage.conf` and prints:

```text
Using manual MIRAGE_FEATURES override instead of mirage.conf
```

`KERNEL_FEATURES` and `QEMU_FEATURES` remain as backward-compatible fallbacks, but `mirage.conf` is the primary path.

## Make targets

Configuration targets:

- `make mirageconfig` — open the menu and regenerate artifacts.
- `make defconfig` — create `mirage.conf` from defaults and regenerate artifacts.
- `make oldconfig` — migrate/update `mirage.conf` and regenerate artifacts.
- `make savedefconfig` — write `mirage.defconfig` with non-default values.
- `make listconfig` — list all schema options.
- `make checkconfig` or `make config-check` — validate and regenerate artifacts.
- `make config-generate` — ensure `mirage.conf` exists, validate/update it, and emit generated artifacts.
- `make config-print` — print derived Cargo and QEMU settings.

Build targets that depend on validated/generated config artifacts include:

- `make build`
- `make kernel`
- `make qemu-kernel`
- `make image`
- `make qemu`
- `make qemu-headless`
- `make qemu-debug`

## QEMU integration

The QEMU scripts source `target/mirage/config/build_flags.env` when present.

Configuration effects:

- `CONFIG_MIRAGE_HW_FRAMEBUFFER=y` selects graphical launch behavior by leaving display args empty.
- `CONFIG_MIRAGE_HW_FRAMEBUFFER=n` derives `-display none`.
- `CONFIG_MIRAGE_QEMU_DEBUG=y` derives `-S -s -d int,cpu_reset -D build/qemu.log`.
- `CONFIG_MIRAGE_SERIAL_CONSOLE=y` derives `-serial stdio`.
- `CONFIG_MIRAGE_VERBOSE_BOOT=y` derives `MIRAGE_KERNEL_CMDLINE="mirage.verbose=1"` for boot metadata consumers.

Integrated scripts:

- `tools/run-qemu.sh`
- `tools/run-qemu-headless.sh`
- `tools/run-qemu-debug.sh`
- `tools/build-qemu-image.sh`

`run-qemu-debug.sh` still forces debug launch flags for the explicit debug entry point, while normal QEMU launches follow the generated config.

## Sample configs

Sample complete configs live under `configs/`:

- `configs/qemu-minimal.mirage.conf` — minimal QEMU-oriented config with no framebuffer/full boot.
- `configs/qemu-framebuffer.mirage.conf` — default QEMU framebuffer config.
- `configs/ryzen-dev.mirage.conf` — Ryzen/PCI/storage/debug development config.
- `configs/full-boot.mirage.conf` — full boot with verbose debug intent.

Use a sample by copying it to `mirage.conf`:

```sh
cp configs/ryzen-dev.mirage.conf mirage.conf
make oldconfig
```

## Clean behavior

`make clean` removes build artifacts and generated config artifacts, but never removes `mirage.conf`.

`make distclean` also preserves `mirage.conf` by default. To explicitly remove the user configuration:

```sh
make distclean CONFIG_CLEAN=1
```

The build system must never silently delete user configuration.
