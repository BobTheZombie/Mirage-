#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

mkdir -p "$repo_root/build"

printf 'Connect with: gdb -ex "target remote :1234"\n' >&2
printf 'CPU reset/interrupt logs are written to build/qemu.log\n' >&2

exec "$script_dir/run-qemu.sh" -S -s -d int,cpu_reset -D build/qemu.log "$@"
