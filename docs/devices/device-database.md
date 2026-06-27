# Mirage Device Database

The Mirage device database is the reviewed source of hardware identity data used to generate small target-side lookup tables. It exists to keep device matching repeatable without turning the kernel into a vendor-specific monolith.

## Goals

- Keep PCI, USB, CPU/chipset, block, char, and input identifiers in auditable data files.
- Generate `no_std` Rust tables for early boot and driver-selection code.
- Preserve Mirage architecture boundaries: data may suggest drivers and quirks, but the Supervisor owns service policy and the kernel validates real hardware state.
- Make external imports traceable through license and provenance fields.

## TOML rationale

Source database files should be TOML because TOML is human-readable, stable in diffs, easy to review, and supports straightforward schema validation in host tooling. TOML is for maintainers and build tools only.

## No TOML in the kernel

The Mirage kernel must not parse TOML, link a TOML parser, allocate parser data structures, or trust unvalidated source database files at runtime. Target code consumes generated `no_std` Rust tables or compact generated binary tables only. This keeps early boot deterministic, small, and suitable for `no_std` environments.

## Source layout

Recommended layout:

```text
device-db/
├── pci/
│   ├── vendors.toml
│   ├── devices.toml
│   └── quirks.toml
├── usb/
│   ├── vendors.toml
│   ├── devices.toml
│   └── interfaces.toml
├── cpu-chipset/
│   ├── x86_64.toml
│   └── quirks.toml
├── block-char-input/
│   ├── block.toml
│   ├── char.toml
│   └── input.toml
└── schema/
    └── device-descriptor.schema.toml
```

Generated output should live outside the reviewed source data, for example:

```text
kernel/generated/device_tables.rs
```

## Generated `no_std` tables

Generated tables must use fixed-width integers, static slices, and simple enums that work in `no_std`. They should avoid heap allocation, string parsing, filesystem access, or policy callbacks. A table entry may include display strings when useful for diagnostics, but matching must use numeric identity fields.

## Fallback behavior

A database miss is not fatal. Mirage should continue with safe generic drivers, degraded operation, or an explicit unsupported state. A database hit only means a possible match; the driver still has to probe real hardware and report exact failure if initialization does not complete.

## Regeneration commands

The expected workflow is:

```sh
cargo xtask device-db validate
cargo xtask device-db generate --out kernel/generated/device_tables.rs
cargo test -p device-db-tools
```

When tools are not yet implemented, documentation and audits must say so explicitly rather than pretending generated tables are complete.
