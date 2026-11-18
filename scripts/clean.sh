#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

rm -rf "$ROOT_DIR/build"
(cd "$ROOT_DIR" && cargo clean)
(cd "$ROOT_DIR/kernel" && cargo clean)
echo "[clean] Cleaned build outputs and Cargo targets."
