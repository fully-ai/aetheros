# Third-Party Notices

AetherOS uses the following external software components during image generation, testing, or protocol interfacing:

## Limine Bootloader
- **Project**: Limine Bootloader (`https://github.com/limine-bootloader/limine`)
- **License**: BSD-2-Clause (bootloader binaries and installer) / 0BSD (`limine.h` protocol specification)
- **Usage**: Bootloader binaries (`BOOTX64.EFI`, `limine-bios.sys`) fetched during reproducible boot-image assembly (`scripts/image.sh`) from pinned `v9.x-binary` commit `ee5d29cd0a8034612dcd1df3f00052480db785c5`. The kernel implements Limine Boot Protocol v3 structures directly in Rust based on the 0BSD-licensed `limine.h` specification.

## QEMU & EDK2/OVMF
- **Project**: QEMU (`https://www.qemu.org/`) and TianoCore EDK2 OVMF (`https://github.com/tianocore/edk2`)
- **License**: GPL-2.0 / BSD-2-Clause-Patent
- **Usage**: Host-side virtualized x86_64 execution and automated testing environment only. Not linked into AetherOS binaries.

## Rust Core & Alloc Libraries
- **Project**: The Rust Programming Language (`https://www.rust-lang.org/`)
- **License**: MIT / Apache-2.0
- **Usage**: Standard `core` and `alloc` libraries compiled for `x86_64-unknown-none`.
