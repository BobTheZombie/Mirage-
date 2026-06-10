#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

error() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

command -v qemu-system-x86_64 >/dev/null 2>&1 || error "missing required command 'qemu-system-x86_64'"

cd "$repo_root"

iso_image=${MIRAGE_ISO_IMAGE:-build/mirage.iso}
if [ "${MIRAGE_SKIP_BUILD:-0}" != "1" ]; then
    "$script_dir/build-qemu-image.sh"
fi
[ -f "$iso_image" ] || error "missing QEMU image '$iso_image' (run tools/build-qemu-image.sh or unset MIRAGE_SKIP_BUILD)"

mem=${MIRAGE_QEMU_MEMORY:-512M}

accel_args=''
if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
    accel_args='-enable-kvm -cpu host'
else
    if qemu-system-x86_64 -cpu help 2>/dev/null | grep ' max' >/dev/null 2>&1; then
        accel_args='-cpu max'
    else
        accel_args='-cpu qemu64'
    fi
fi

# shellcheck disable=SC2086
exec qemu-system-x86_64 \
    -M q35 \
    -m "$mem" \
    $accel_args \
    -cdrom "$iso_image" \
    -serial stdio \
    -no-reboot \
    -no-shutdown \
    "$@"
