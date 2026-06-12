#!/bin/sh
set -eu

usage() {
    cat <<'USAGE'
Usage: tools/verify-seed-rs-elf.sh [BUILT_KERNEL [STAGED_KERNEL]]

Verifies key Mirage seed-rs kernel ELF handoff properties and prints diagnostic data.

Arguments:
  BUILT_KERNEL  Kernel ELF produced by the Rust build.
                Default: target/x86_64-mirage/release/mirage-kernel
  STAGED_KERNEL Kernel copy staged into the bootable ISO/image tree.
                Default: build/iso_root/boot/mirage-kernel

Environment overrides:
  MIRAGE_KERNEL      Overrides the built kernel default path.
  MIRAGE_STAGED_KERNEL  Overrides the staged kernel copy default path.

Checks performed:
  * prints the ELF entry point from `readelf -h`
  * prints symbol addresses for `_start`, `__mirage_x86_64_seed_entry`,
    optional `__mirage_x86_64_bootstrap`, and `kernel_main` using `nm -n`
  * reports whether Limine request sections are present via `readelf -S`
  * verifies `_start` calls `__mirage_x86_64_seed_entry` instead of the old
    direct bootstrap path
  * prints SHA256 hashes for the built kernel and staged kernel copy
  * exits nonzero if `_start`, `__mirage_x86_64_seed_entry`, or `kernel_main`
    is missing, the ELF entry point differs from `_start`, `_start` does not
    dispatch to seed-rs, any Limine request section is missing, or the built
    and staged kernel hashes differ
USAGE
}

info() {
    printf 'info: %s\n' "$*"
}

warn() {
    printf 'warn: %s\n' "$*" >&2
}

error() {
    printf 'error: %s\n' "$*" >&2
}

have_cmd() {
    command -v "$1" >/dev/null 2>&1
}

require_cmd() {
    if ! have_cmd "$1"; then
        error "required command not found: $1"
        exit 1
    fi
}

normalize_hex() {
    # Lowercase, strip an optional 0x prefix, and remove leading zeroes so
    # readelf's entry-point format can be compared with nm's symbol values.
    value=$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')
    value=${value#0x}
    value=$(printf '%s' "$value" | sed 's/^0*//')
    if [ -z "$value" ]; then
        value=0
    fi
    printf '%s' "$value"
}

symbol_addr() {
    file=$1
    symbol=$2

    # nm output is usually: ADDRESS TYPE SYMBOL. Keep the first exact match.
    nm -n "$file" 2>/dev/null | awk -v sym="$symbol" '$3 == sym { print $1; exit }'
}

section_present() {
    file=$1
    section=$2

    # readelf -SW output contains the section name as a whitespace-delimited
    # field, followed by type/address/offset metadata.
    readelf -SW "$file" | awk -v sec="$section" '{ for (i = 1; i <= NF; i++) if ($i == sec) found = 1 } END { exit found ? 0 : 1 }'
}

case "${1:-}" in
    -h|--help)
        usage
        exit 0
        ;;
esac

if [ "$#" -gt 2 ]; then
    usage >&2
    exit 2
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

built_kernel=${1:-${MIRAGE_KERNEL:-target/x86_64-mirage/release/mirage-kernel}}
iso_kernel=${2:-${MIRAGE_STAGED_KERNEL:-build/iso_root/boot/mirage-kernel}}

case $built_kernel in
    /*) ;;
    *) built_kernel=$repo_root/$built_kernel ;;
esac

case $iso_kernel in
    /*) ;;
    *) iso_kernel=$repo_root/$iso_kernel ;;
esac

require_cmd readelf
require_cmd nm
require_cmd sha256sum
require_cmd objdump

[ -f "$built_kernel" ] || { error "built kernel not found: $built_kernel"; exit 1; }
[ -f "$iso_kernel" ] || { error "staged kernel copy not found: $iso_kernel"; exit 1; }

failures=0

info "built kernel: $built_kernel"
info "staged kernel copy: $iso_kernel"

entry_line=$(readelf -h "$built_kernel" | awk -F: '/Entry point address:/ { gsub(/^[ \t]+/, "", $2); print $2; exit }')
if [ -z "$entry_line" ]; then
    error "could not read ELF entry point from: $built_kernel"
    exit 1
fi
printf 'ELF entry point: %s\n' "$entry_line"
entry_norm=$(normalize_hex "$entry_line")

printf '\nSymbol addresses:\n'
start_addr=$(symbol_addr "$built_kernel" _start || true)
seed_entry_addr=$(symbol_addr "$built_kernel" __mirage_x86_64_seed_entry || true)
bootstrap_addr=$(symbol_addr "$built_kernel" __mirage_x86_64_bootstrap || true)
kernel_main_addr=$(symbol_addr "$built_kernel" kernel_main || true)

if [ -n "$start_addr" ]; then
    printf '  _start: %s\n' "$start_addr"
else
    printf '  _start: MISSING\n'
    error "required symbol missing: _start"
    failures=$((failures + 1))
fi

if [ -n "$seed_entry_addr" ]; then
    printf '  __mirage_x86_64_seed_entry: %s\n' "$seed_entry_addr"
else
    printf '  __mirage_x86_64_seed_entry: MISSING\n'
    error "required symbol missing: __mirage_x86_64_seed_entry"
    failures=$((failures + 1))
fi

if [ -n "$bootstrap_addr" ]; then
    printf '  __mirage_x86_64_bootstrap: %s\n' "$bootstrap_addr"
else
    printf '  __mirage_x86_64_bootstrap: MISSING\n'
    info "optional symbol not found: __mirage_x86_64_bootstrap"
fi

if [ -n "$kernel_main_addr" ]; then
    printf '  kernel_main: %s\n' "$kernel_main_addr"
else
    printf '  kernel_main: MISSING\n'
    error "required symbol missing: kernel_main"
    failures=$((failures + 1))
fi

if [ -n "$start_addr" ]; then
    start_norm=$(normalize_hex "$start_addr")
    if [ "$entry_norm" = "$start_norm" ]; then
        info "ELF entry point matches _start"
    else
        error "ELF entry point ($entry_line) does not equal _start (0x$start_addr)"
        failures=$((failures + 1))
    fi
fi


printf '\n_start dispatch:\n'
start_disassembly=$(objdump -d --disassemble=_start "$built_kernel" 2>/dev/null || true)
if printf '%s\n' "$start_disassembly" | grep -q '__mirage_x86_64_seed_entry'; then
    info "_start dispatches to __mirage_x86_64_seed_entry"
elif printf '%s\n' "$start_disassembly" | grep -q '__mirage_x86_64_bootstrap'; then
    error "_start still dispatches to __mirage_x86_64_bootstrap instead of seed-rs"
    failures=$((failures + 1))
else
    error "could not prove _start dispatches to __mirage_x86_64_seed_entry"
    failures=$((failures + 1))
fi

if [ -n "$bootstrap_addr" ]; then
    if printf '%s\n' "$start_disassembly" | grep -q '__mirage_x86_64_bootstrap'; then
        error "compatibility bootstrap symbol is still used by _start"
        failures=$((failures + 1))
    else
        warn "__mirage_x86_64_bootstrap still exists as an unused compatibility symbol"
    fi
fi

printf '\nLimine request sections:\n'
for section in .requests_start_marker .requests .requests_end_marker; do
    if section_present "$built_kernel" "$section"; then
        printf '  %s: present\n' "$section"
    else
        printf '  %s: MISSING\n' "$section"
        error "Limine request section not found: $section"
        failures=$((failures + 1))
    fi
done

printf '\nSHA256 hashes:\n'
built_hash=$(sha256sum "$built_kernel" | awk '{ print $1 }')
iso_hash=$(sha256sum "$iso_kernel" | awk '{ print $1 }')
printf '  built kernel: %s  %s\n' "$built_hash" "$built_kernel"
printf '  staged kernel: %s  %s\n' "$iso_hash" "$iso_kernel"

if [ "$built_hash" = "$iso_hash" ]; then
    info "built kernel hash matches staged kernel hash"
else
    error "built kernel hash differs from staged kernel hash"
    failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
    error "seed-rs ELF verification failed with $failures failure(s)"
    exit 1
fi

info "seed-rs ELF verification passed"
