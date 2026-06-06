#!/usr/bin/env sh
set -eu

# Build and boot the Mirage ISO in QEMU, then validate the serial boot log.
#
# Environment overrides:
#   MAKE              make command to use (default: make)
#   QEMU              QEMU binary to use (default: qemu-system-x86_64)
#   ISO_IMAGE         ISO path to boot (default: build/mirage.iso)
#   TIMEOUT_SECONDS   QEMU runtime before forced stop (default: 20)
#   SMOKE_LOG         serial log path (default: build/qemu-smoke.log)

MAKE=${MAKE:-make}
QEMU=${QEMU:-qemu-system-x86_64}
ISO_IMAGE=${ISO_IMAGE:-build/mirage.iso}
TIMEOUT_SECONDS=${TIMEOUT_SECONDS:-20}
SMOKE_LOG=${SMOKE_LOG:-build/qemu-smoke.log}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'error: required command not found: %s\n' "$1" >&2
        exit 127
    fi
}

require_log_entry() {
    needle=$1
    if ! grep -Fq "$needle" "$SMOKE_LOG"; then
        printf 'error: expected serial output not found: %s\n' "$needle" >&2
        printf '%s\n' '--- captured serial output ---' >&2
        cat "$SMOKE_LOG" >&2 || true
        printf '%s\n' '--- end captured serial output ---' >&2
        exit 1
    fi
}

require_command "$MAKE"
require_command "$QEMU"
require_command timeout

"$MAKE" iso

mkdir -p "$(dirname "$SMOKE_LOG")"
: > "$SMOKE_LOG"

set +e
timeout --foreground "$TIMEOUT_SECONDS" "$QEMU" \
    -M q35 \
    -m 256M \
    -cdrom "$ISO_IMAGE" \
    -serial stdio \
    -monitor none \
    -display none \
    -no-reboot \
    -no-shutdown >"$SMOKE_LOG" 2>&1
qemu_status=$?
set -e

# timeout(1) exits 124 after stopping QEMU. That is expected because the
# current boot milestone intentionally reaches the idle loop and keeps running.
if [ "$qemu_status" -ne 0 ] && [ "$qemu_status" -ne 124 ]; then
    printf 'error: QEMU exited with status %s\n' "$qemu_status" >&2
    printf '%s\n' '--- captured serial output ---' >&2
    cat "$SMOKE_LOG" >&2 || true
    printf '%s\n' '--- end captured serial output ---' >&2
    exit "$qemu_status"
fi

require_log_entry 'Mirage kernel booting'
require_log_entry 'Mirage reached idle loop'

printf 'qemu smoke test passed; serial log: %s\n' "$SMOKE_LOG"
