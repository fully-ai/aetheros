#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
echo "[debug] Starting QEMU paused with GDB stub on tcp::1234..."
echo "[debug] Connect from another terminal with:"
echo "        gdb $ROOT_DIR/build/aetheros.elf -ex 'target remote :1234'"
exec env AETHEROS_NO_KVM=1 "$ROOT_DIR/scripts/run.sh" -s -S "$@"
