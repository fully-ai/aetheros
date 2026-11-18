# ADR-0004: Native System Call ABI (AetherOS ABI v1)

- **Status**: Accepted
- **Date**: 2026-11-18
- **Decision Makers**: AetherOS Core Engineering

## Context

AetherOS requires a versioned, clean, auditable system call interface between Ring 3 user programs and the Ring 0 kernel. It must NOT falsely claim Linux binary compatibility before a separate compatibility layer exists.

## Decision

Define **AetherOS Native Syscall ABI v1** using the x86_64 `SYSCALL`/`SYSRET` mechanism:
- **Syscall number**: `RAX`
- **Arguments 1..6**: `RDI`, `RSI`, `RDX`, `R10`, `R8`, `R9` (`RCX` and `R11` are clobbered by hardware `SYSCALL` to store user `RIP` and `RFLAGS`)
- **Return value**: `RAX` — non-negative (`>= 0`) indicates success; negative values in `[-4095, -1]` indicate `-Errno`
- **ABI Discovery**: `sys_abi_version` returns ABI major/minor version and kernel capabilities
- **Error Discipline**: Every unimplemented or planned syscall returns `-ENOSYS`. No stub syscall ever returns fake `0` success.
- **Pointer Validation**: Every user pointer and buffer range `(ptr, len)` is checked for overflow, canonical user-space bounds (`< USER_ADDR_MAX`), and mapped page permissions before dereferencing.

## Consequences

- Register convention mirrors standard System V / x86_64 `SYSCALL` ergonomics, simplifying future C/Rust toolchain integration and a distinct, opt-in Linux compatibility personality in a later milestone.
- Shared `aether-abi` crate guarantees exact constant and struct layout parity between kernel and userspace.
