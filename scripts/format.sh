#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

MODE="${1:-fmt}"
if [[ "$MODE" == "--check" ]]; then
    (cd "$ROOT_DIR" && cargo fmt --all -- --check)
    (cd "$ROOT_DIR/kernel" && cargo fmt --all -- --check)
    (cd "$ROOT_DIR/userspace" && cargo fmt --all -- --check)
else
    (cd "$ROOT_DIR" && cargo fmt --all)
    (cd "$ROOT_DIR/kernel" && cargo fmt --all)
    (cd "$ROOT_DIR/userspace" && cargo fmt --all)
fi
