#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
cd "$repo_root"

iso_image=${MIRAGE_ISO_IMAGE:-build/mirage.iso}
runtime_image=${MIRAGE_BOOT_RUNTIME_IMAGE:-build/spider-rt.img}
iso_root=${MIRAGE_ISO_ROOT:-build/iso_root}

fail=0
err() { printf 'error: %s\n' "$*" >&2; fail=1; }
need_file() { [ -f "$1" ] || err "missing host artifact: $1"; }

need_file "$runtime_image"

check_runtime_path() {
    path=$1
    # The runtime image stores path names in fixed metadata records. Match either
    # the mounted absolute path required by the boot contract or the historical
    # mount-relative path accepted by RuntimeVfs.
    rel=${path#/spider-rt}
    if ! strings "$runtime_image" | grep -Fx -- "$path" >/dev/null 2>&1 \
        && ! strings "$runtime_image" | grep -Fx -- "$rel" >/dev/null 2>&1; then
        err "boot runtime image '$runtime_image' is missing $path"
    else
        printf 'found runtime: %s\n' "$path"
    fi
}

check_runtime_path /spider-rt/sbin/spider-rs
check_runtime_path /spider-rt/sbin/spider-rsd

check_root_path_in_tree() {
    path=$1
    [ -f "$iso_root$path" ] || err "rootfs staging '$iso_root' is missing $path"
    [ ! -e "$iso_root/spider-rt${path#/usr}" ] || err "normal userspace app appears under /spider-rt: /spider-rt${path#/usr}"
    [ ! -e "$iso_root/spider-rt/sbin/m1-terminal" ] || err "m1-terminal must not be packaged under /spider-rt"
    [ -f "$iso_root$path" ] && printf 'found rootfs staging: %s\n' "$path"
}

check_root_path_in_iso() {
    path=$1
    if command -v xorriso >/dev/null 2>&1 && [ -f "$iso_image" ]; then
        xorriso -indev "$iso_image" -find "$path" -print 2>/dev/null | grep -Fx -- "$path" >/dev/null 2>&1 \
            || err "ISO '$iso_image' is missing $path"
        if xorriso -indev "$iso_image" -find /spider-rt/sbin/m1-terminal -print 2>/dev/null | grep -Fx /spider-rt/sbin/m1-terminal >/dev/null 2>&1; then
            err "ISO '$iso_image' incorrectly contains /spider-rt/sbin/m1-terminal"
        fi
    elif [ -f "$iso_image" ]; then
        printf 'warning: xorriso unavailable; skipping ISO rootfs inspection for %s\n' "$path" >&2
    fi
}

for path in \
    /usr/bin/m1-terminal \
    /etc/spider/units/default.target \
    /etc/spider/units/basic.target \
    /etc/spider/units/m1-terminal.service
 do
    check_root_path_in_tree "$path"
    check_root_path_in_iso "$path"
 done

if [ -f "$iso_image" ] && command -v xorriso >/dev/null 2>&1; then
    xorriso -indev "$iso_image" -find /spider-rt.img -print 2>/dev/null | grep -Fx /spider-rt.img >/dev/null 2>&1 \
        || err "ISO '$iso_image' is missing /spider-rt.img"
    [ "$fail" -eq 0 ] && printf 'found ISO module: /spider-rt.img\n'
fi

[ "$fail" -eq 0 ] || exit 1
printf 'boot runtime validation passed\n'
