#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

printf 'QEMU will start paused with a GDB stub on TCP port 1234.\n' >&2
printf 'Connect from gdb with: target remote :1234\n' >&2

exec "$script_dir/run-qemu.sh" -S -s "$@"
