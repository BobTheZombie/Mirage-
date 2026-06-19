# Spider unit files

Spider units use Mirage-native INI syntax. Supported sections are `[Unit]`, `[Target]`, `[Service]`, and `[Install]`. Supported dependencies are `Requires=`, `Wants=`, and `After=`. Supported service restart policies are `Restart=no`, `Restart=on-failure`, and `Restart=always`.

Runtime search paths are `/etc/spider/units` and `/usr/lib/spider/units`, with `/run/spider/units` reserved for generated units. If directory enumeration is unavailable, spider-rsd may use compiled built-in unit text, parsed by the same parser.

The first service unit is `/etc/spider/units/m1-terminal.service` and uses `ExecStart=/usr/bin/m1-terminal`.
