#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

PROFILE="${AETHEROS_PROFILE:-release}"
CARGO_FLAGS=()
if [[ "$PROFILE" == "release" ]]; then
    CARGO_FLAGS+=(--release)
    TARGET_SUBDIR="release"
else
    TARGET_SUBDIR="debug"
fi

for tool in cargo rustc; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "ERROR: required tool '$tool' not found in PATH" >&2
        exit 1
    fi
done

echo "[build] Building host workspace crates..."
(cd "$ROOT_DIR" && cargo build --workspace)

echo "[build] Building AetherOS Ring 3 ELF64 user-space binaries..."
(cd "$ROOT_DIR/userspace" && cargo build --release)

USERSPACE_OUT="$ROOT_DIR/userspace/target/x86_64-unknown-none/release"
USERSPACE_ROOT="$ROOT_DIR/build/userspace_root"
rm -rf "$USERSPACE_ROOT"
mkdir -p "$USERSPACE_ROOT/bin" "$USERSPACE_ROOT/sbin"

cp "$USERSPACE_OUT/init" "$USERSPACE_ROOT/sbin/init"
cp "$USERSPACE_OUT/aethersh" "$USERSPACE_ROOT/bin/aethersh"
cp "$USERSPACE_OUT/sudo" "$USERSPACE_ROOT/bin/sudo"
cp "$USERSPACE_OUT/aetherpkg" "$USERSPACE_ROOT/bin/aetherpkg"
cp "$USERSPACE_OUT/coreutils" "$USERSPACE_ROOT/sbin/useradd"

for util in ls cat grep wc ps kill id whoami groups stat chmod chown mkdir rmdir touch rm cp mv uname uptime free df dmesg mount umount; do
    cp "$USERSPACE_OUT/coreutils" "$USERSPACE_ROOT/bin/$util"
done

echo "[build] Building AetherOS x86_64 kernel ($PROFILE)..."
(cd "$ROOT_DIR/kernel" && cargo build "${CARGO_FLAGS[@]}")

mkdir -p "$ROOT_DIR/build"
cp "$ROOT_DIR/kernel/target/x86_64-unknown-none/$TARGET_SUBDIR/aetheros" "$ROOT_DIR/build/aetheros.elf"

echo "[build] Kernel ELF ready: $ROOT_DIR/build/aetheros.elf"
ls -lh "$ROOT_DIR/build/aetheros.elf"

"$ROOT_DIR/scripts/image.sh"
