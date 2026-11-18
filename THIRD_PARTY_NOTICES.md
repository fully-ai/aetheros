# Third-Party Notices

AetherOS is an independent operating system engineered from the ground up. It uses the following open-source bootloader protocol crate and boot binary distribution during disk image packaging:

1. **`limine` Rust Boot Protocol Crate (`v0.5.0`)**:
   - Purpose: Zero-dependency `#![no_std]` data structures for the Limine Boot Protocol v3 handoff (`kernel/Cargo.toml`).
   - License: BSD-2-Clause / MIT.
2. **Limine Bootloader Binaries (`v9.x-binary`, commit `ee5d29cd0a8034612dcd1df3f00052480db785c5`)**:
   - Purpose: `BOOTX64.EFI`, `limine-bios.sys`, and `limine-bios-hdd.bin` packaged into `build/aetheros.img` by `scripts/image.sh` (not committed as binary blobs in the source tree).
   - Upstream: `https://github.com/limine-bootloader/limine`
   - License: BSD-2-Clause.
