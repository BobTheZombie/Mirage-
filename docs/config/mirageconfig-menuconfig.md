# MirageConfig menuconfig interface

`mirageconfig` is Mirage's host-side configuration utility. The `--menu` and `--menuconfig` modes provide a Linux `menuconfig`-style interface with expandable categories and selectable options.

## Commands

```sh
make menuconfig
make mirageconfig
cargo run -q -p mirageconfig -- --menu --config mirage.conf --generate
```

## Controls

| Key | Action |
| --- | --- |
| Up / Down | Move selection |
| j / k | Move selection |
| Enter / Right | Expand category or toggle selected option |
| Left | Collapse category |
| Space | Toggle selected option |
| / | Search symbols, prompts, categories, and help text |
| c | Clear search |
| ? / h | Toggle help pane |
| s | Save and exit |
| q | Quit; prompt to save when dirty |

## Schema fields

`config/MirageConfig.toml` remains the source of truth. Each option supports:

```toml
[[options]]
symbol = "CONFIG_MIRAGE_EXAMPLE"
prompt = "Example option"
category = "Kernel Core"
default = false
help = "Describe what this option does."
depends_on = []
selects = []
visible_if = []
type = "bool"
cargo_feature = "example-feature"
```

Supported types:

- `bool`
- `tristate`
- `string`
- `int`
- `hex`

Current Mirage options are mostly bools, but the tool is ready for richer schema entries.

## Output

Configs are written in menuconfig-style format:

```text
CONFIG_MIRAGE_FULL_BOOT=y
# CONFIG_MIRAGE_DEBUG is not set
```

Generated artifacts remain under:

```text
target/mirage/config/
```

The generated environment files are safe to source from the root `Makefile`.
