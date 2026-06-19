#!/usr/bin/env bash
set -euo pipefail
LOG=${1:-build/boot-order.log}
mkdir -p "$(dirname "$LOG")"
: > "$LOG"
timeout 60s tools/run-qemu.sh | tee "$LOG"
grep -q "kernel constructed" "$LOG"
grep -q "boot info applied" "$LOG"
if ! grep -Eq "MTSS initialized|MTSS online" "$LOG"; then
  echo "boot-order check failed: MTSS marker missing" >&2
  exit 1
fi
