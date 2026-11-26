# AetherOS Testing & Verification Architecture

See also [TESTING_AND_VERIFICATION.md](TESTING_AND_VERIFICATION.md) and [TEST_STATUS.md](TEST_STATUS.md).

## 1. Two-Tier Verification Pipeline (`./scripts/test.sh`)

1. **Tier 1 — Host Workspace Unit & Fuzz Tests (`cargo test --workspace`)**:
   - `aether-abi` (2 tests): Packed ABI layout & negative `Errno` encoding.
   - `aether-uuid` (4 tests): RFC 4122 UUID v4 parser, formatter, root/nil invariants, deterministic generator.
   - `aether-elf` (4 tests): Valid ELF64 parsing, kernel-address segment rejection, `W^X` segment rejection, and **10,000-iteration mutated ELF64 header fuzz test**.
   - `aether-fs-core` (4 tests): **2 PiB volume format + sparse file write/read at `1 PiB + 8 KiB`**, Primary Superblock (`LBA 1`) corruption failover to Backup (`LBA 2`), Write-Ahead Journal (`AETHJRNL`) committed replay & torn-write rollback, and 256-byte Inode CRC32C tamper detection.
2. **Tier 2 — Automated QEMU UEFI (OVMF) & Legacy BIOS In-Kernel + Ring 3 Tests**:
   - Boots `build/aetheros-test.img` with `cmdline: console=ttyS0 selftest=all` in both UEFI (`OVMF`) and Legacy BIOS.
   - Runs 19 comprehensive kernel and Ring 3 user-space self-test suites (`kernel/src/selftest.rs`) and exits QEMU via `isa-debug-exit` (`0xF4`) with exit code `33` (`PASS`).
