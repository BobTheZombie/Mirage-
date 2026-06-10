#!/usr/bin/env sh
set -eu

# Build or inspect the Mirage x86_64 kernel ELF, then validate the non-emulator
# boot artifact baseline: entry point, architecture/class, required linker
# symbols, and retained Limine request sections.
#
# Environment overrides:
#   MAKE           make command to use when building (default: make)
#   KERNEL_ELF     kernel ELF to inspect (default: target/x86_64-mirage/release/mirage-kernel)
#   BUILD_KERNEL   run "$MAKE kernel" before inspection when set to 1 (default: 1 for default ELF, 0 for custom KERNEL_ELF)
#   READELF        readelf-compatible tool to use (default: readelf or llvm-readelf)

MAKE=${MAKE:-make}
DEFAULT_KERNEL_ELF=target/x86_64-mirage/release/mirage-kernel
if [ -n "${KERNEL_ELF:-}" ]; then
    BUILD_KERNEL=${BUILD_KERNEL:-0}
else
    BUILD_KERNEL=${BUILD_KERNEL:-1}
    KERNEL_ELF=$DEFAULT_KERNEL_ELF
fi

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'error: required command not found: %s\n' "$1" >&2
        exit 127
    fi
}

choose_readelf() {
    if [ "${READELF+x}" = x ]; then
        require_command "$READELF"
        printf '%s\n' "$READELF"
        return
    fi

    if command -v readelf >/dev/null 2>&1; then
        printf '%s\n' readelf
        return
    fi

    if command -v llvm-readelf >/dev/null 2>&1; then
        printf '%s\n' llvm-readelf
        return
    fi

    printf '%s\n' 'error: required command not found: readelf or llvm-readelf' >&2
    exit 127
}

fail() {
    printf 'error: %s\n' "$1" >&2
    exit 1
}

symbol_value() {
    symbol=$1
    "$READELF_TOOL" -Ws "$KERNEL_ELF" | awk -v symbol="$symbol" '$NF == symbol { print $2; found = 1; exit } END { if (!found) exit 1 }'
}

require_symbol() {
    symbol=$1
    if ! value=$(symbol_value "$symbol"); then
        fail "required symbol not found in ELF symbol table: $symbol"
    fi

    case "$value" in
        ''|0000000000000000)
            fail "required symbol has an invalid zero value: $symbol"
            ;;
    esac
}

require_section() {
    section=$1
    if ! "$READELF_TOOL" -SW "$KERNEL_ELF" | awk -v section="$section" '{ for (i = 1; i <= NF; i++) if ($i == section) found = 1 } END { exit(found ? 0 : 1) }'; then
        fail "required linked section not retained: $section"
    fi
}

entry_point() {
    "$READELF_TOOL" -h "$KERNEL_ELF" | awk -F: '/Entry point address:/ { gsub(/^[[:space:]]+/, "", $2); print tolower($2); found = 1; exit } END { if (!found) exit 1 }'
}

header_value() {
    label=$1
    "$READELF_TOOL" -h "$KERNEL_ELF" | awk -F: -v label="$label" '$1 ~ label { gsub(/^[[:space:]]+/, "", $2); print $2; found = 1; exit } END { if (!found) exit 1 }'
}

READELF_TOOL=$(choose_readelf)

if [ "$BUILD_KERNEL" = 1 ]; then
    require_command "$MAKE"
    if [ "$KERNEL_ELF" = "$DEFAULT_KERNEL_ELF" ]; then
        rm -f "$KERNEL_ELF"
    fi
    "$MAKE" kernel
fi

if [ ! -f "$KERNEL_ELF" ]; then
    fail "kernel ELF not found: $KERNEL_ELF"
fi

elf_class=$(header_value 'Class')
case "$elf_class" in
    ELF64*) ;;
    *) fail "unexpected ELF class: $elf_class" ;;
esac

elf_machine=$(header_value 'Machine')
case "$elf_machine" in
    *X86-64*|*x86-64*|*Advanced\ Micro\ Devices\ X86-64*) ;;
    *) fail "unexpected ELF machine: $elf_machine" ;;
esac

start_value=$(symbol_value _start || true)
if [ -z "$start_value" ]; then
    fail 'required entry symbol not found in ELF symbol table: _start'
fi

start_entry=$(entry_point)
start_value=$(printf '0x%s\n' "$start_value" | tr '[:upper:]' '[:lower:]')
if [ "$start_entry" != "$start_value" ]; then
    fail "ELF entry point $start_entry does not match _start symbol $start_value"
fi

for symbol in \
    _start \
    __mirage_x86_64_bootstrap \
    __limine_requests_start \
    __limine_requests_end \
    __stack_top \
    __bss_start \
    __bss_end
    do
    require_symbol "$symbol"
done

for section in \
    .requests \
    .requests_start_marker \
    .requests_end_marker
    do
    require_section "$section"
done

printf 'x86_64 boot artifact smoke test passed: %s\n' "$KERNEL_ELF"
printf '  readelf tool: %s\n' "$READELF_TOOL"
printf '  entry point: %s (_start)\n' "$start_entry"
printf '  class: %s\n' "$elf_class"
printf '  machine: %s\n' "$elf_machine"
