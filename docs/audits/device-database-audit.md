# Device Database Audit

This audit tracks the status of Mirage device database design, generation, validation, and provenance controls.

## Current status

The documentation defines the intended contract and schema expectations. If the generator, schema validator, or generated `no_std` table output is absent, the device database must be treated as design-stage work and not as boot acceptance evidence.

## Audit checklist

- Source data uses TOML for reviewability and schema validation.
- TOML parsing is excluded from kernel and target boot images.
- Generated tables are `no_std`, deterministic, and reproducible.
- PCI, USB, CPU/chipset, block, char, and input fields have explicit schemas.
- Driver hints are not treated as Supervisor authorization.
- Unknown devices have safe fallback behavior.
- External imports include license and provenance metadata.
- Regeneration commands are documented and tested.
- Positive and negative schema tests exist.
- Runtime status distinguishes database match from real driver initialization and ONLINE state.

## Regeneration evidence to capture

Record the exact commands and output for:

```sh
cargo xtask device-db validate
cargo xtask device-db generate --out kernel/generated/device_tables.rs
cargo test -p device-db-tools
```

## License and provenance rules

External PCI, USB, CPU, chipset, block, char, and input identifiers may be imported only after license review. The audit must name inspected files or pages, summarize what was learned, identify license terms, and state whether Mirage copied data, transformed data, or used observation-derived facts. GPL or otherwise incompatible implementation code must not be copied into Mirage.

## Open risks

- Generator and schema tooling may not yet exist.
- Device identity data can become stale without a documented update cadence.
- Ambiguous matches can select unsafe drivers unless priority and fallback rules are tested.
- A database hit may be mistaken for hardware readiness unless boot UI and logs keep states separate.
