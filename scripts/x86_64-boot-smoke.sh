#!/usr/bin/env sh
set -eu

# Build or inspect the Mirage x86_64 kernel ELF, then validate the Limine
# handoff contract without starting an emulator. The audit checks the entry
# point, required bootstrap/linker symbols, retained Limine request sections,
# request declarations in src/boot.rs, binary request fingerprints, and the
# kernel staged into the ISO root.
#
# Environment overrides:
#   MAKE           make command to use when building (default: make)
#   KERNEL_ELF     kernel ELF to inspect (default: target/x86_64-mirage/release/mirage-kernel)
#   ISO_KERNEL     ISO-staged kernel to compare (default: build/iso_root/boot/mirage-kernel)
#   BUILD_KERNEL   run "$MAKE kernel" before inspection when set to 1 (default: 1 for default ELF, 0 for custom KERNEL_ELF)
#   READELF        readelf-compatible tool to use (default: readelf or llvm-readelf)
#   NM             nm-compatible tool to use (default: nm or llvm-nm)
#   SHA256SUM      sha256sum-compatible tool to use (default: sha256sum or shasum -a 256)

MAKE=${MAKE:-make}
DEFAULT_KERNEL_ELF=target/x86_64-mirage/release/mirage-kernel
BOOT_SOURCE=${BOOT_SOURCE:-src/boot.rs}
ISO_KERNEL=${ISO_KERNEL:-build/iso_root/boot/mirage-kernel}
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

choose_nm() {
    if [ "${NM+x}" = x ]; then
        require_command "$NM"
        printf '%s\n' "$NM"
        return
    fi

    if command -v nm >/dev/null 2>&1; then
        printf '%s\n' nm
        return
    fi

    if command -v llvm-nm >/dev/null 2>&1; then
        printf '%s\n' llvm-nm
        return
    fi

    printf '%s\n' 'error: required command not found: nm or llvm-nm' >&2
    exit 127
}

choose_sha256() {
    if [ "${SHA256SUM+x}" = x ]; then
        require_command "$SHA256SUM"
        printf '%s\n' "$SHA256SUM"
        return
    fi

    if command -v sha256sum >/dev/null 2>&1; then
        printf '%s\n' sha256sum
        return
    fi

    if command -v shasum >/dev/null 2>&1; then
        printf '%s\n' 'shasum -a 256'
        return
    fi

    printf '%s\n' 'error: required command not found: sha256sum or shasum' >&2
    exit 127
}

fail() {
    printf 'error: %s\n' "$1" >&2
    exit 1
}

symbol_value() {
    symbol=$1
    awk -v symbol="$symbol" '$NF == symbol { print $2; found = 1; exit } END { if (!found) exit 1 }' "$SYMBOLS_OUT"
}

nm_symbol_value() {
    symbol=$1
    awk -v symbol="$symbol" '$NF == symbol { print $1; found = 1; exit } END { if (!found) exit 1 }' "$NM_OUT"
}

require_symbol() {
    symbol=$1
    if ! value=$(symbol_value "$symbol"); then
        fail "required symbol not found in readelf symbol table: $symbol"
    fi

    case "$value" in
        ''|0000000000000000)
            fail "required symbol has an invalid zero value: $symbol"
            ;;
    esac

    if ! nm_value=$(nm_symbol_value "$symbol"); then
        fail "required symbol not found in nm -n output: $symbol"
    fi

    readelf_value=$(printf '0x%s\n' "$value" | tr '[:upper:]' '[:lower:]')
    nm_value=$(printf '0x%s\n' "$nm_value" | tr '[:upper:]' '[:lower:]')
    if [ "$readelf_value" != "$nm_value" ]; then
        fail "symbol $symbol differs between readelf ($readelf_value) and nm ($nm_value)"
    fi
}

section_line() {
    section=$1
    awk -v section="$section" '{ for (i = 1; i <= NF; i++) if ($i == section) { print; found = 1; exit } } END { if (!found) exit 1 }' "$SECTIONS_OUT"
}

section_addr() {
    section=$1
    section_line "$section" | awk '{ print "0x" $5 }'
}

section_size_hex() {
    section=$1
    section_line "$section" | awk '{ print $7 }'
}

require_section() {
    section=$1
    if ! section_line "$section" >/dev/null; then
        fail "required linked section not retained: $section"
    fi
}

entry_point() {
    awk -F: '/Entry point address:/ { gsub(/^[[:space:]]+/, "", $2); print tolower($2); found = 1; exit } END { if (!found) exit 1 }' "$HEADER_OUT"
}

header_value() {
    label=$1
    awk -F: -v label="$label" '$1 ~ label { gsub(/^[[:space:]]+/, "", $2); print $2; found = 1; exit } END { if (!found) exit 1 }' "$HEADER_OUT"
}

source_request_count() {
    awk '
        /#\[link_section = "\.requests"\]/ { in_requests = 1; next }
        in_requests && /^[[:space:]]*(pub[[:space:]]+)?static([[:space:]]+mut)?[[:space:]]+[A-Z0-9_]+[[:space:]:=]/ {
            count++;
            in_requests = 0;
            next;
        }
        in_requests && /^#\[/ { next }
        in_requests && NF { in_requests = 0 }
        END { print count + 0 }
    ' "$BOOT_SOURCE"
}

require_source_request() {
    request=$1
    awk -v request="$request" '
        /#\[link_section = "\.requests"\]/ { in_requests = 1; next }
        in_requests && $0 ~ "^[[:space:]]*(pub[[:space:]]+)?static([[:space:]]+mut)?[[:space:]]+" request "[[:space:]:=]" { found = 1; exit }
        in_requests && /^#\[/ { next }
        in_requests && NF { in_requests = 0 }
        END { exit(found ? 0 : 1) }
    ' "$BOOT_SOURCE" || fail "request $request is not declared in $BOOT_SOURCE with #[link_section = \".requests\"]"
}

require_hex_fingerprint() {
    name=$1
    first=$2
    second=$3
    if ! awk -v first="$first" -v second="$second" 'BEGIN { found_first = 0 } $0 ~ first { found_first = 1 } found_first && $0 ~ second { found = 1; exit } END { exit(found ? 0 : 1) }' "$REQUESTS_HEX_OUT"; then
        fail "binary .requests evidence not found for $name"
    fi
}

sha256_of() {
    # shellcheck disable=SC2086
    $SHA256_TOOL "$1" | awk '{ print $1 }'
}

size_of() {
    wc -c < "$1" | awk '{ print $1 }'
}

READELF_TOOL=$(choose_readelf)
NM_TOOL=$(choose_nm)
SHA256_TOOL=$(choose_sha256)

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

if [ ! -f "$BOOT_SOURCE" ]; then
    fail "boot request source not found: $BOOT_SOURCE"
fi

TMPDIR=${TMPDIR:-/tmp}
AUDIT_TMP=$(mktemp -d "$TMPDIR/mirage-handoff-audit.XXXXXX")
trap 'rm -rf "$AUDIT_TMP"' EXIT HUP INT TERM
HEADER_OUT=$AUDIT_TMP/readelf-h.txt
SECTIONS_OUT=$AUDIT_TMP/readelf-SW.txt
SYMBOLS_OUT=$AUDIT_TMP/readelf-sW.txt
NM_OUT=$AUDIT_TMP/nm-n.txt
REQUESTS_HEX_OUT=$AUDIT_TMP/readelf-x-requests.txt

"$READELF_TOOL" -h "$KERNEL_ELF" > "$HEADER_OUT"
"$READELF_TOOL" -SW "$KERNEL_ELF" > "$SECTIONS_OUT"
"$READELF_TOOL" -sW "$KERNEL_ELF" > "$SYMBOLS_OUT"
"$NM_TOOL" -n "$KERNEL_ELF" > "$NM_OUT"
"$READELF_TOOL" -x .requests "$KERNEL_ELF" > "$REQUESTS_HEX_OUT"

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
    __mirage_x86_64_seed_entry \
    kernel_main \
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

requests_addr=$(section_addr .requests)
requests_size_hex=$(section_size_hex .requests)
requests_size_nonzero=$(printf '%s\n' "$requests_size_hex" | tr -d '0')
if [ -z "$requests_size_nonzero" ]; then
    fail '.requests section has zero size'
fi

# Verify source declarations for the handoff requests Mirage currently depends on.
for request in \
    BASE_REVISION \
    FRAMEBUFFER \
    MEMORY_MAP \
    HHDM \
    RSDP \
    EXECUTABLE_ADDRESS \
    MODULES
    do
    require_source_request "$request"
done

expected_request_count=$(source_request_count)
if [ "$expected_request_count" -lt 7 ]; then
    fail "unexpectedly low request count from $BOOT_SOURCE: $expected_request_count"
fi

# Verify binary fingerprints for every request object declared in .requests. The
# hex words are Limine request IDs as encoded in src/boot.rs; readelf displays
# bytes in little-endian order, so each u64 appears as two reversed-endian u32s.
require_hex_fingerprint BASE_REVISION 'c8a6955c 2d2b56f9' 'dc6b5344 49387b6a'
require_hex_fingerprint BOOTLOADER_INFO '2f20a1e2 d83850f5' '4097f5f5 fc269427'
require_hex_fingerprint STACK_SIZE '26898e0a 46f04e22' '3dea465f c20fcbe1'
require_hex_fingerprint HHDM '52b8d28a cbf1dc48' '4b24989a 954e9863'
require_hex_fingerprint FRAMEBUFFER '75dd81d8 dc27589d' '1bb1faf6 048614a3'
require_hex_fingerprint MEMORY_MAP '6f808a37 9d3dcf67' '623c0cc5 dfac04e3'
require_hex_fingerprint RSDP '437b7e39 6b7be7c5' '3ccfcdac 45786327'
require_hex_fingerprint EXECUTABLE_ADDRESS '635fc53c 8676ba71' '87a416c5 484a64b2'
require_hex_fingerprint MODULES 'af32be02 97277e3e' 'ee0c28d1 3b4f1cca'

if [ "$expected_request_count" -ne 9 ]; then
    fail "expected request count from $BOOT_SOURCE changed to $expected_request_count; update audit fingerprints if intentional"
fi

if [ ! -f "$ISO_KERNEL" ]; then
    fail "ISO-staged kernel not found: $ISO_KERNEL"
fi

kernel_size=$(size_of "$KERNEL_ELF")
iso_kernel_size=$(size_of "$ISO_KERNEL")
if [ "$kernel_size" != "$iso_kernel_size" ]; then
    fail "kernel size mismatch: built=$kernel_size iso=$iso_kernel_size"
fi

kernel_sha=$(sha256_of "$KERNEL_ELF")
iso_kernel_sha=$(sha256_of "$ISO_KERNEL")
if [ "$kernel_sha" != "$iso_kernel_sha" ]; then
    fail "kernel SHA256 mismatch: built=$kernel_sha iso=$iso_kernel_sha"
fi

printf 'x86_64 Limine handoff audit passed: %s\n' "$KERNEL_ELF"
printf '  ELF entry: %s\n' "$start_entry"
printf '  _start: %s\n' "$start_value"
printf '  .requests address: %s\n' "$requests_addr"
printf '  request count: %s\n' "$expected_request_count"
printf '  built kernel SHA256: %s\n' "$kernel_sha"
printf '  ISO-staged kernel SHA256: %s\n' "$iso_kernel_sha"
