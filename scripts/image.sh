#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

LIMINE_DIR="$ROOT_DIR/build/limine"
LIMINE_COMMIT="ee5d29cd0a8034612dcd1df3f00052480db785c5"
KERNEL_ELF="$ROOT_DIR/build/aetheros.elf"
IMG_PATH="${AETHEROS_IMG:-$ROOT_DIR/build/aetheros.img}"
ESP_IMG="$ROOT_DIR/build/esp.img"
INITRAMFS_TAR="$ROOT_DIR/build/initramfs.tar"
CMDLINE="${AETHEROS_CMDLINE:-console=ttyS0}"

for tool in git cc dd parted mkfs.fat mmd mcopy tar; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "ERROR: required image-building tool '$tool' not found in PATH" >&2
        exit 1
    fi
done

if [[ ! -f "$KERNEL_ELF" ]]; then
    echo "ERROR: kernel ELF not found at $KERNEL_ELF. Run scripts/build.sh first." >&2
    exit 1
fi

# Fetch pinned Limine v9.x-binary release if not already cached
if [[ ! -x "$LIMINE_DIR/limine" ]]; then
    echo "[image] Fetching pinned Limine v9.x-binary ($LIMINE_COMMIT)..."
    rm -rf "$LIMINE_DIR"
    mkdir -p "$LIMINE_DIR"
    git clone --branch v9.x-binary --single-branch --depth 1 \
        https://github.com/limine-bootloader/limine.git "$LIMINE_DIR"
    ACTUAL_COMMIT="$(git -C "$LIMINE_DIR" rev-parse HEAD)"
    if [[ "$ACTUAL_COMMIT" != "$LIMINE_COMMIT" ]]; then
        echo "[image] Note: v9.x-binary HEAD is $ACTUAL_COMMIT (expected $LIMINE_COMMIT)"
    fi
    cc -O2 -Wall "$LIMINE_DIR/limine.c" -o "$LIMINE_DIR/limine"
fi

# Assemble initramfs.tar (USTAR format)
INITRAMFS_STAGE="$ROOT_DIR/build/initramfs_root"
rm -rf "$INITRAMFS_STAGE"
mkdir -p "$INITRAMFS_STAGE/etc" "$INITRAMFS_STAGE/bin" "$INITRAMFS_STAGE/sbin"
cat > "$INITRAMFS_STAGE/etc/os-release" <<'EOF'
NAME="AetherOS"
ID=aetheros
VERSION="0.1.0-alpha.1"
PRETTY_NAME="AetherOS 0.1.0-alpha.1 (x86_64)"
EOF

if [[ -d "$ROOT_DIR/build/userspace_root" ]]; then
    cp -a "$ROOT_DIR/build/userspace_root/." "$INITRAMFS_STAGE/"
fi

tar --format=ustar -cf "$INITRAMFS_TAR" -C "$INITRAMFS_STAGE" .

# Generate effective limine.conf
LIMINE_CONF="$ROOT_DIR/build/limine.conf"
cat > "$LIMINE_CONF" <<EOF
timeout: 0
serial: yes

/AetherOS
    protocol: limine
    path: boot():/boot/aetheros.elf
    module_path: boot():/boot/initramfs.tar
    cmdline: $CMDLINE
EOF

# Build 64 MiB hybrid GPT disk image (UEFI ESP + Limine BIOS boot)
echo "[image] Creating hybrid UEFI + BIOS GPT image: $IMG_PATH"
rm -f "$IMG_PATH" "$ESP_IMG"
dd if=/dev/zero of="$IMG_PATH" bs=1M count=64 status=none
parted -s "$IMG_PATH" mklabel gpt
parted -s "$IMG_PATH" mkpart ESP fat32 2048s 129023s
parted -s "$IMG_PATH" set 1 esp on

# 126976 sectors = 62 MiB FAT32 partition image
dd if=/dev/zero of="$ESP_IMG" bs=512 count=126976 status=none
mkfs.fat -F 32 -n AETHEROS "$ESP_IMG" >/dev/null

mmd -D o -i "$ESP_IMG" ::/EFI
mmd -D o -i "$ESP_IMG" ::/EFI/BOOT
mmd -D o -i "$ESP_IMG" ::/boot
mmd -D o -i "$ESP_IMG" ::/boot/limine

mcopy -D o -i "$ESP_IMG" "$LIMINE_DIR/BOOTX64.EFI" ::/EFI/BOOT/
mcopy -D o -i "$ESP_IMG" "$LIMINE_DIR/limine-bios.sys" ::/boot/limine/
mcopy -D o -i "$ESP_IMG" "$LIMINE_CONF" ::/boot/limine/
mcopy -D o -i "$ESP_IMG" "$KERNEL_ELF" ::/boot/
mcopy -D o -i "$ESP_IMG" "$INITRAMFS_TAR" ::/boot/

# Write FAT32 partition at sector 2048 (1 MiB offset)
dd if="$ESP_IMG" of="$IMG_PATH" bs=512 seek=2048 conv=notrunc status=none

# Install Limine BIOS bootloader stages into GPT MBR + stage2 gap
"$LIMINE_DIR/limine" bios-install "$IMG_PATH" >/dev/null

echo "[image] Bootable hybrid image ready: $IMG_PATH"
