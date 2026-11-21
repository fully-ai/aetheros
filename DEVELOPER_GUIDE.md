# AetherOS Developer Guide (`DEVELOPER_GUIDE.md`)

See also [BUILD_AND_QEMU_GUIDE.md](file:///home/ubeyt/Belgeler/AetherOS/BUILD_AND_QEMU_GUIDE.md) and [DEVELOPMENT.md](file:///home/ubeyt/Belgeler/AetherOS/DEVELOPMENT.md).

## 1. Workspace Layout
- `crates/aether-abi`: Shared `no_std` Syscall ABI v1 numbers, `Errno`, `AetherStat`, `Dirent64`, `PollFd`, `ProcInfo`.
- `crates/aether-uuid`: RFC 4122 128-bit UUID v4 type and parser.
- `crates/aether-fs-core`: Portable `no_std` AetherFS v1 (`>= 1 PiB` 64-bit extents, dual superblocks, CRC32C, `AETHJRNL` WAL journal).
- `crates/aether-elf`: Fuzz-tested `no_std` x86_64 ELF64 parser & validator.
- `kernel/`: Higher-half `x86_64-unknown-none` kernel (`arch`, `boot`, `drivers`, `fs`, `gui`, `ipc`, `mm`, `net`, `sched`, `security`, `storage`, `sync`, `syscall`, `usb`, `selftest`).
- `userspace/`: Freestanding Ring 3 ELF64 runtime (`libaether`) and binaries (`init`, `aethersh`, `coreutils`, `sudo`, `aetherpkg`).

## 2. Verification & Debugging Workflow
```bash
# Format & Clippy lint across all 3 workspaces:
./scripts/format.sh --check
./scripts/lint.sh

# Run 14 host unit/fuzz tests + 21 QEMU UEFI & Legacy BIOS self-tests:
./scripts/test.sh

# Debug kernel with GDB on localhost:1234:
./scripts/debug.sh
```
