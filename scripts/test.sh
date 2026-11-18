#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

echo "=== [1/3] Running Host Workspace Unit Tests ==="
(cd "$ROOT_DIR" && cargo test --workspace)

echo "=== [2/3] Building Kernel & Self-Test Boot Image ==="
AETHEROS_CMDLINE="console=ttyS0 selftest=all" AETHEROS_IMG="$ROOT_DIR/build/aetheros-test.img" \
    "$ROOT_DIR/scripts/build.sh"

run_qemu_selftest() {
    local mode="$1"
    local log_file="$ROOT_DIR/build/qemu-selftest-${mode}.log"
    echo "=== [3/3] Running Automated QEMU Self-Test (mode=$mode) ==="
    set +e
    timeout 30s env \
        AETHEROS_IMG="$ROOT_DIR/build/aetheros-test.img" \
        AETHEROS_BOOT_MODE="$mode" \
        AETHEROS_DISPLAY=none \
        AETHEROS_DEBUG_EXIT=1 \
        "$ROOT_DIR/scripts/run.sh" </dev/null >"$log_file" 2>&1
    local rc=$?
    set -e

    cat "$log_file"

    # isa-debug-exit code 0x10 produces exit code ((0x10 << 1) | 1) = 33
    if [[ "$rc" -ne 33 ]]; then
        echo "ERROR: QEMU self-test ($mode) FAILED with exit code $rc (expected 33)" >&2
        exit 1
    fi
    if ! grep -q "\[AETHEROS-TEST\] ALL .* PASSED" "$log_file"; then
        echo "ERROR: QEMU self-test ($mode) did not emit PASS banner" >&2
        exit 1
    fi
    echo "[PASS] QEMU self-test ($mode) succeeded (exit=33)"
}

run_qemu_selftest uefi
run_qemu_selftest bios

echo "=== ALL AETHEROS TESTS PASSED ==="
