#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

echo "[lint] Running clippy on host workspace..."
(cd "$ROOT_DIR" && cargo clippy --workspace --all-targets -- -D warnings)

echo "[lint] Running clippy on x86_64 kernel..."
(cd "$ROOT_DIR/kernel" && cargo clippy -- -D warnings)
