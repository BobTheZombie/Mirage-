#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

info() {
    printf 'info: %s\n' "$*"
}

skip() {
    printf 'skip: %s\n' "$*"
    exit 0
}

error() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

have_cmd() {
    command -v "$1" >/dev/null 2>&1
}

require_repo_file() {
    [ -f "$repo_root/$1" ] || error "missing required repository file: $1"
}

missing_tools=''
check_tool() {
    tool=$1
    reason=$2

    if have_cmd "$tool"; then
        info "found $reason: $(command -v "$tool")"
    else
        printf 'skip: missing %s (%s)\n' "$tool" "$reason"
        missing_tools="${missing_tools}${missing_tools:+ }$tool"
    fi
}

cd "$repo_root"

info 'checking shell syntax for tools/*.sh'
sh -n tools/*.sh || error 'shell syntax check failed for tools/*.sh'

require_repo_file Cargo.toml
require_repo_file src/main.rs
require_repo_file boot/limine/limine.conf
require_repo_file targets/x86_64-mirage.json

make_cmd=${MAKE:-make}
cargo_cmd=${CARGO:-cargo}
rustc_cmd=${RUSTC:-rustc}

check_tool "$make_cmd" 'make, required to drive the image build and build Limine'
check_tool "$cargo_cmd" 'cargo, required to build the Mirage kernel'
check_tool "$rustc_cmd" 'rustc, required to verify build-std Rust source availability'
check_tool curl 'curl, required to fetch the Limine binary release when not cached'
check_tool tar 'tar, required to unpack Limine'
check_tool xorriso 'xorriso, required to assemble the bootable ISO image'

if [ -n "$missing_tools" ]; then
    skip "QEMU image build not run because required host tool(s) are unavailable: $missing_tools"
fi

sysroot=$($rustc_cmd --print sysroot 2>/dev/null) || skip "could not query Rust sysroot with '$rustc_cmd --print sysroot'"
rust_src_lock=$sysroot/lib/rustlib/src/rust/library/Cargo.lock
if [ ! -f "$rust_src_lock" ]; then
    skip "Rust source for -Z build-std is unavailable at $rust_src_lock; install rust-src before running the image build"
fi
info "found Rust source for build-std: $rust_src_lock"

info 'building Mirage QEMU image without launching QEMU'
"$make_cmd" image

iso_image=${MIRAGE_ISO_IMAGE:-build/mirage.iso}
[ -f "$iso_image" ] || error "expected ISO image was not created: $iso_image"

info "QEMU image build check passed: $iso_image"
