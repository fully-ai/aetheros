#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

IMG_PATH="${AETHEROS_IMG:-$ROOT_DIR/build/aetheros.img}"
BOOT_MODE="${AETHEROS_BOOT_MODE:-uefi}"
MEMORY="${AETHEROS_MEM:-256M}"
CPUS="${AETHEROS_CPUS:-2}"
DISPLAY_MODE="${AETHEROS_DISPLAY:-none}"

if ! command -v qemu-system-x86_64 >/dev/null 2>&1; then
    echo "ERROR: qemu-system-x86_64 not found in PATH" >&2
    exit 1
fi

if [[ ! -f "$IMG_PATH" ]]; then
    echo "[run] Boot image missing; running scripts/build.sh..."
    "$ROOT_DIR/scripts/build.sh"
fi

QEMU_ARGS=(
    -m "$MEMORY"
    -smp "$CPUS"
    -drive "file=$IMG_PATH,format=raw,if=ide"
    -serial stdio
    -no-reboot
)

if [[ -r /dev/kvm && -w /dev/kvm && "${AETHEROS_NO_KVM:-0}" != "1" ]]; then
    QEMU_ARGS+=(-accel kvm -cpu host)
else
    QEMU_ARGS+=(-cpu qemu64,+smep,+smap)
fi

if [[ "$DISPLAY_MODE" == "none" ]]; then
    QEMU_ARGS+=(-display none)
fi

if [[ "$BOOT_MODE" == "uefi" ]]; then
    OVMF_CODE=""
    OVMF_VARS_SRC=""
    for candidate in \
        "$HOME/.local/qemu/share/qemu/edk2-x86_64-code.fd" \
        "/usr/share/edk2/x64/OVMF_CODE.4m.fd" \
        "/usr/share/OVMF/OVMF_CODE.fd" \
        "/usr/share/qemu/edk2-x86_64-code.fd"; do
        if [[ -f "$candidate" ]]; then
            OVMF_CODE="$candidate"
            break
        fi
    done
    for vcandidate in \
        "$HOME/.local/qemu/share/qemu/edk2-i386-vars.fd" \
        "/usr/share/edk2/x64/OVMF_VARS.4m.fd" \
        "/usr/share/OVMF/OVMF_VARS.fd" \
        "/usr/share/qemu/edk2-i386-vars.fd"; do
        if [[ -f "$vcandidate" ]]; then
            OVMF_VARS_SRC="$vcandidate"
            break
        fi
    done
    if [[ -z "$OVMF_CODE" ]]; then
        echo "ERROR: OVMF UEFI firmware (edk2-x86_64-code.fd) not found. Try AETHEROS_BOOT_MODE=bios." >&2
        exit 1
    fi
    QEMU_ARGS+=(-drive "if=pflash,format=raw,unit=0,readonly=on,file=$OVMF_CODE")
    if [[ -n "$OVMF_VARS_SRC" ]]; then
        OVMF_VARS_COPY="$ROOT_DIR/build/ovmf_vars.fd"
        cp "$OVMF_VARS_SRC" "$OVMF_VARS_COPY"
        QEMU_ARGS+=(-drive "if=pflash,format=raw,unit=1,file=$OVMF_VARS_COPY")
    fi
fi

if [[ "${AETHEROS_DEBUG_EXIT:-0}" == "1" ]]; then
    QEMU_ARGS+=(-device isa-debug-exit,iobase=0xf4,iosize=0x04)
fi

if [[ -n "${AETHEROS_DISK:-}" && -f "${AETHEROS_DISK:-}" ]]; then
    QEMU_ARGS+=(-drive "file=$AETHEROS_DISK,format=raw,if=virtio")
fi

exec qemu-system-x86_64 "${QEMU_ARGS[@]}" "$@"
