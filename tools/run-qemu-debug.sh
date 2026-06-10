#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

printf 'Connect with: gdb -ex "target remote :1234"\n' >&2

exec "$script_dir/run-qemu.sh" -S -s "$@"
