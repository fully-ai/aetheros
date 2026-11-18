# AetherOS Current Engineering State

- **Version**: `v0.1.0-alpha.1`
- **Target Architecture**: `x86_64-unknown-none` (64-bit Long Mode, Ring 0 Higher-Half Kernel at `0xffffffff80000000`, Ring 3 User Space at `0x0000000000400000..0x00007fffffff0000`)
- **Boot Targets Verified**: QEMU `x86_64` **UEFI (EDK2 OVMF)** and **Legacy BIOS** (hybrid GPT image built via `./scripts/make-image.sh`)
- **Active Milestones Completed**: Milestones **0 through 14** (100% of planned v0.1.0 milestones)
- **Formatting & Linting**:
  - `./scripts/format.sh --check` passes cleanly across host workspace, `kernel/`, and `userspace/`.
  - `./scripts/lint.sh` (`cargo clippy -- -D warnings`) passes with **0 warnings** across all 3 workspaces.
- **Automated Verification (`./scripts/test.sh`)**:
  - **14 / 14** Host Workspace Unit & Fuzz Smoke Tests passing.
  - **19 / 19** In-Kernel + Ring 3 Self-Test Suites passing in **QEMU UEFI** (`exit=33`) and **QEMU Legacy BIOS** (`exit=33`).
