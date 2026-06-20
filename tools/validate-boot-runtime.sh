#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
cd "$repo_root"

ISO=${MIRAGE_ISO_IMAGE:-build/mirage.iso}
runtime_image=${MIRAGE_BOOT_RUNTIME_IMAGE:-build/spider-rt.img}
runtime_tree=${MIRAGE_BOOT_RUNTIME_TREE:-build/spider-rt}
iso_root=${MIRAGE_ISO_ROOT:-build/iso_root}
rootfs_staging=${MIRAGE_ROOTFS_STAGING:-build/rootfs}

fail=0
iso_file_list=
runtime_file_list=

cleanup() {
    [ -z "$iso_file_list" ] || rm -f "$iso_file_list"
    [ -z "$runtime_file_list" ] || rm -f "$runtime_file_list"
}
trap cleanup EXIT INT HUP TERM

err() {
    printf 'error: %s\n' "$*" >&2
    fail=1
}

require_host_file() {
    path=$1
    label=$2
    if [ ! -f "$path" ]; then
        err "missing $label: $path"
        return 1
    fi
    printf 'found %s: %s\n' "$label" "$path"
    return 0
}

require_absent_host_path() {
    path=$1
    reason=$2
    if [ -e "$path" ]; then
        err "$reason: $path"
        return 1
    fi
    return 0
}

build_runtime_file_list() {
    runtime_file_list=$(mktemp)
    if ! cargo run -q -p mk-runtime-image -- --list "$runtime_image" >"$runtime_file_list"; then
        err "failed to inspect boot runtime image '$runtime_image'"
        : >"$runtime_file_list"
    fi
}

build_iso_file_list() {
    iso_file_list=$(mktemp)
    raw_list=$(mktemp)
    err_list=$(mktemp)
    if ! command -v xorriso >/dev/null 2>&1; then
        err "xorriso is required to inspect ISO Rock Ridge/POSIX paths"
        : >"$iso_file_list"
        rm -f "$raw_list" "$err_list"
        return
    fi

    # Newer xorriso builds accept the documented -print action. Some packaged
    # versions print matching paths by default and reject -print. Support both,
    # but only accept exact absolute POSIX/Rock Ridge paths after normalization.
    if ! xorriso -indev "$ISO" -find / -type f -print >"$raw_list" 2>"$err_list"; then
        if ! xorriso -indev "$ISO" -find / -type f >"$raw_list" 2>"$err_list"; then
            cat "$err_list" >&2
            err "failed to inspect ISO '$ISO' with xorriso"
            : >"$iso_file_list"
            rm -f "$raw_list" "$err_list"
            return
        fi
    fi

    sed -n "s/^'\(\/.*\)'$/\1/p; /^\/.*$/p" "$raw_list" | sort -u >"$iso_file_list"
    rm -f "$raw_list" "$err_list"
}

require_list_path() {
    list_file=$1
    container=$2
    path=$3
    if ! grep -Fxq -- "$path" "$list_file"; then
        err "$container is missing $path"
        return 1
    fi
    printf 'found %s path: %s\n' "$container" "$path"
    return 0
}

require_iso_absent() {
    path=$1
    reason=$2
    if grep -Fxq -- "$path" "$iso_file_list"; then
        err "$reason: $path"
        return 1
    fi
    return 0
}

# Stage 1: expanded spider runtime staging.
for path in \
    /spider-rt/sbin/spider-rs \
    /spider-rt/sbin/spider-rsd
 do
    require_host_file "$runtime_tree$path" "runtime staging"
 done
require_absent_host_path "$runtime_tree/spider-rt/sbin/m1-terminal" "m1-terminal must not be packaged under /spider-rt"

# Stage 2: packed runtime image; inspect the real image manifest, not just staging.
require_host_file "$runtime_image" "boot runtime image"
build_runtime_file_list
for path in \
    /spider-rt/sbin/spider-rs \
    /spider-rt/sbin/spider-rsd
 do
    require_list_path "$runtime_file_list" "runtime image '$runtime_image'" "$path"
 done

# Stage 3: rootfs staging.
for path in \
    /usr/bin/m1-terminal \
    /etc/spider/units/default.target \
    /etc/spider/units/basic.target \
    /etc/spider/units/m1-terminal.service
 do
    require_host_file "$rootfs_staging$path" "rootfs staging"
 done
require_absent_host_path "$rootfs_staging/spider-rt/sbin/m1-terminal" "m1-terminal must not be packaged under /spider-rt"

# Stage 4: ISO root staging.
for path in \
    /spider-rt.img \
    /usr/bin/m1-terminal \
    /etc/spider/units/default.target \
    /etc/spider/units/basic.target \
    /etc/spider/units/m1-terminal.service
 do
    require_host_file "$iso_root$path" "ISO root staging"
 done
require_absent_host_path "$iso_root/spider-rt/sbin/m1-terminal" "m1-terminal must not be packaged under /spider-rt"

# Stage 5: generated ISO, using Rock Ridge/POSIX paths from xorriso.
require_host_file "$ISO" "ISO image"
build_iso_file_list
for path in \
    /spider-rt.img \
    /usr/bin/m1-terminal \
    /etc/spider/units/default.target \
    /etc/spider/units/basic.target \
    /etc/spider/units/m1-terminal.service
 do
    require_list_path "$iso_file_list" "ISO '$ISO'" "$path"
 done
require_iso_absent /spider-rt/sbin/m1-terminal "ISO '$ISO' incorrectly contains m1-terminal under /spider-rt"

[ "$fail" -eq 0 ] || exit 1
printf 'boot runtime validation passed\n'
