# ADR-0002: Implementation Languages — Stable Rust (`no_std`) + x86_64 Assembly

- **Status**: Accepted
- **Date**: 2026-11-18
- **Decision Makers**: AetherOS Core Engineering

## Context

AetherOS requires memory safety, explicit error handling, strict subsystem boundaries, and zero-overhead access to x86_64 hardware primitives, while supporting both bare-metal kernel compilation and host-side unit/fuzz testing.

## Decision

1. **Kernel**: Written in **stable Rust** targeting `x86_64-unknown-none` (`no_std` + `alloc`), using `core::arch::asm!` and `core::arch::global_asm!` for boot-critical and architecture-specific operations (IDT trap stubs, context switching, `SYSCALL`/`SYSRET` entry trampolines, `iretq` Ring 3 transitions, MSR/CR/port I/O).
2. **Shared Core Crates (`crates/*`)**: `aether-abi`, `aether-uuid`, `aether-elf`, and `aether-fs-core` are written in `no_std` Rust so the exact same implementation compiles into the kernel AND executes exhaustive host-side unit, crash-recovery, and fuzz tests under `cargo test`.
3. **Userspace (`userspace/*`)**: Native static ELF64 binaries written in `no_std` Rust with a clean `libaether` runtime and syscall interface, avoiding premature libc bloat while maintaining POSIX-inspired ergonomics.
4. **Host Tooling**: Rust (`mkfs-aetherfs`, `aether-test-runner`) and POSIX Bash scripts (`scripts/*.sh`).

## Rationale for Avoiding Nightly-Only `x86-interrupt`

Rather than depending on unstable `abi_x86_interrupt`, all 256 IDT entries route through explicit x86_64 assembly stubs that push a uniform `TrapFrame` onto the kernel stack and call a `sysv64` Rust dispatcher. This:
- Keeps the entire OS building on **100% stable Rust**.
- Provides uniform register access for exception diagnostics, signal delivery, and preemptive context switching directly from timer interrupts.

## Consequences

- Reproducible builds pinned via `rust-toolchain.toml` (stable `1.100.0`).
- Zero undefined behavior from red-zone corruption or SSE register clobbering (`x86_64-unknown-none` disables red zone and SSE/MMX by default).
- Every `unsafe` block is isolated to hardware boundaries, page tables, allocator internals, and FFI/ABI edges with explicit safety comments.
