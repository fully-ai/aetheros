# AetherOS Build, QEMU Execution & GDB Debugging Guide

## 1. Host Prerequisites

- **Rust Toolchain (`rustup`)**:
  ```bash
  rustup toolchain install stable
  rustup target add x86_64-unknown-none
  rustup component add rust-src llvm-tools-preview rustfmt clippy
  ```
- **System Packages**:
  - `qemu-system-x86_64` (with EDK2 OVMF firmware `edk2-x86_64-code.fd` / `OVMF.fd`)
  - `mtools` (`mmd`, `mcopy`), `dosfstools` (`mkfs.vfat`), `parted` or `sgdisk`, `tar`

---

## 2. Standard Engineering Scripts (`scripts/`)

| Script | Command | Action |
| :--- | :--- | :--- |
| **Build All** | `./scripts/build.sh` | Compiles host shared crates, compiles all Ring 3 ELF64 binaries (`userspace/`), and builds the higher-half `x86_64` kernel ELF (`build/aetheros.elf`) |
| **Create Disk Image** | `./scripts/make-image.sh` | Assembles `build/initramfs.tar` (36 Ring 3 binaries under `/sbin` and `/bin`) and constructs a 64 MiB hybrid UEFI + Legacy BIOS GPT disk image (`build/aetheros.img`) using Limine v9 |
| **Run in QEMU (UEFI)** | `./scripts/run-qemu.sh` | Boots `build/aetheros.img` under OVMF UEFI firmware with 512 MiB RAM, 2 SMP cores, and serial console attached |
| **Run in QEMU (BIOS)** | `./scripts/run-qemu.sh --bios` | Boots `build/aetheros.img` under Legacy BIOS |
| **Run All Tests** | `./scripts/test.sh` | Runs host unit/fuzz tests (`cargo test --workspace`) + automated QEMU self-tests in **both** UEFI and Legacy BIOS |
| **Debug with GDB** | `./scripts/debug-qemu.sh` | Launches QEMU paused (`-s -S`) listening for GDB on `localhost:1234` |
| **Format Code** | `./scripts/format.sh [--check]` | Runs `cargo fmt` across host workspace, `kernel/`, and `userspace/` |
| **Lint Code** | `./scripts/lint.sh` | Runs `cargo clippy -- -D warnings` across host workspace, `kernel/`, and `userspace/` |
| **Clean Build** | `./scripts/clean.sh` | Removes `build/` and Cargo target artifacts |

---

## 3. Debugging with GDB

1. Start QEMU with the GDB stub paused at reset:
   ```bash
   ./scripts/debug-qemu.sh
   ```
2. In a second terminal, attach `gdb` (or `rust-gdb`) using the unstripped kernel ELF:
   ```bash
   gdb build/aetheros.elf \
     -ex "target remote localhost:1234" \
     -ex "hbreak aetheros::_start" \
     -ex "continue"
   ```
