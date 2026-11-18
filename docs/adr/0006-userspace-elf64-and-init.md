# ADR-0006: Executable Format (ELF64) & User-Space Init Architecture

- **Status**: Accepted
- **Date**: 2026-11-18
- **Decision Makers**: AetherOS Core Engineering

## Context

AetherOS needs a standard, tooling-supported executable format and a real user-space system supervisor (`/sbin/init`, PID 1) rather than kernel-launched daemons.

## Decision

1. **ELF64 Native Format**: Support static x86_64 ELF64 (`ET_EXEC` and later `ET_DYN`/PIE) executables parsed by a dedicated, fuzz-tested, `no_std` validator crate (`aether-elf`). No proprietary executable format is invented prematurely.
2. **Strict Validation**: Every header field, program header count, file offset, `p_filesz <= p_memsz`, page alignment, and user virtual address bound (`< USER_ADDR_MAX`) is validated with checked arithmetic before touching page tables.
3. **User-Space `/sbin/init` (PID 1)**: The kernel mounts root/initramfs and executes `/sbin/init` in Ring 3 as PID 1. Init mounts `/dev`, `/proc`, `/tmp`, `/run`, mounts persistent AetherFS volumes, starts configured services, reaps orphaned zombies, manages logs, and spawns the interactive console / login / `aethersh`.

## Consequences

- Standard Rust/LLVM toolchains emit native AetherOS user binaries directly.
- Kernel stays clean of policy and daemon orchestration.
