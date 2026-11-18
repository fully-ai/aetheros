# AetherOS Developer Workflow Guide

See also [BUILD_AND_QEMU_GUIDE.md](file:///home/ubeyt/Belgeler/AetherOS/BUILD_AND_QEMU_GUIDE.md).

## 1. Standard Developer Commands (`scripts/`)

```bash
# 1. Build host crates, Ring 3 ELF64 userspace, kernel ELF, and hybrid UEFI+BIOS GPT disk image:
./scripts/build.sh

# 2. Boot AetherOS in QEMU (default UEFI via OVMF; pass --bios for Legacy BIOS):
./scripts/run.sh

# 3. Run full automated host + QEMU (UEFI & BIOS) verification suite:
./scripts/test.sh

# 4. Launch QEMU paused with GDB server on localhost:1234:
./scripts/debug.sh

# 5. Format and lint all workspaces (host, kernel, userspace):
./scripts/format.sh --check
./scripts/lint.sh

# 6. Clean build outputs:
./scripts/clean.sh
```

## 2. Pinned Toolchain Versions
- **Rust Compiler**: `rustc 1.100.0` (`stable-x86_64-unknown-linux-gnu`) with `x86_64-unknown-none` target (`rust-toolchain.toml`).
- **Bootloader**: Limine `v9.6.7` (`v9.x-binary` commit `ee5d29cd0a8034612dcd1df3f00052480db785c5`).
- **Emulator**: `qemu-system-x86_64` with EDK2 OVMF (`edk2-x86_64-code.fd`).
