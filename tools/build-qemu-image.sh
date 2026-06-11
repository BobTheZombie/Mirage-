#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

error() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || error "missing required command '$1'"
}

need_cmd cargo
need_cmd rustup
need_cmd xorriso
need_cmd curl
need_cmd tar
need_cmd make

cd "$repo_root"

[ -f Cargo.toml ] || error "could not find Cargo.toml at repo root: $repo_root"
[ -f boot/limine/limine.conf ] || error "missing Limine config: boot/limine/limine.conf"
[ -f targets/x86_64-mirage.json ] || error "missing target JSON: targets/x86_64-mirage.json"

mode=${MIRAGE_BUILD_MODE:-full}
case "$mode" in
    full)
        if [ -n "${MIRAGE_FEATURES:-}" ]; then
            printf 'Using manual MIRAGE_FEATURES override instead of mirage.conf\n'
        fi
        printf 'Building Mirage QEMU ISO from mirage.conf (mode: full)\n'
        make image
        ;;
    minimal)
        if [ -n "${MIRAGE_FEATURES:-}" ]; then
            printf 'Using manual MIRAGE_FEATURES override instead of mirage.conf\n'
            make image
        else
            printf 'Building Mirage QEMU ISO in minimal compatibility mode with features: %s\n' "${QEMU_MINIMAL_FEATURES:-hw-framebuffer}"
            make image QEMU_FEATURES="${QEMU_MINIMAL_FEATURES:-hw-framebuffer}"
        fi
        ;;
    *)
        error "unknown MIRAGE_BUILD_MODE '$mode' (expected 'full' or 'minimal')"
        ;;
esac

iso_image=${MIRAGE_ISO_IMAGE:-build/mirage.iso}
[ -f "$iso_image" ] || error "ISO was not created: $iso_image"

for artifact in \
    build/limine/limine \
    build/limine/limine-bios.sys \
    build/limine/limine-bios-cd.bin \
    build/limine/limine-uefi-cd.bin \
    build/limine/BOOTX64.EFI \
    build/limine/BOOTIA32.EFI
 do
    [ -f "$artifact" ] || error "missing Limine artifact after build: $artifact"
 done

printf 'Mirage QEMU ISO ready: %s\n' "$iso_image"
