# ADR-0003: Virtual Memory Layout — 4-Level Paging & Higher-Half Kernel

- **Status**: Accepted
- **Date**: 2026-11-18
- **Decision Makers**: AetherOS Core Engineering

## Context

AetherOS must strictly isolate Ring 3 user processes from Ring 0 kernel code and data, enforce W^X and NX, and maintain clean separation between user address spaces and shared kernel mappings.

## Decision

Adopt x86_64 4-level paging (48-bit canonical virtual addresses) with the following layout:

| Virtual Address Range | Region | Privilege | Notes |
|---|---|---|---|
| `0x0000_0000_0000_0000` – `0x0000_0000_0000_FFFF` | Null Guard Region | Unmapped | Always unmapped; catches null/near-null dereferences |
| `0x0000_0000_0040_0000` – `0x0000_3FFF_FFFF_FFFF` | User Code, Data, Heap (`brk`) | Ring 3 (U/S=1) | Per-process isolated PML4 lower half (entries 0..255) |
| `0x0000_4000_0000_0000` – `0x0000_7EFF_FFFF_FFFF` | User `mmap` / Shared Memory | Ring 3 (U/S=1) | Demand-paged, W^X enforced |
| `0x0000_7FFF_F000_0000` – `0x0000_7FFF_FFFF_0000` | User Stack (with Guard Page) | Ring 3 (U/S=1, NX) | Grows downward; bottom guard page unmapped |
| `0x0000_8000_0000_0000` – `0xFFFF_7FFF_FFFF_FFFF` | Non-Canonical Hole | Faults (#GP) | Hardware-enforced canonical boundary |
| `0xFFFF_8000_0000_0000` – `0xFFFF_8FFF_FFFF_FFFF` | Higher-Half Direct Map (HHDM) | Ring 0 (U/S=0, NX) | Bootloader-provided physical-to-virtual offset |
| `0xFFFF_9000_0000_0000` – `0xFFFF_9FFF_FFFF_FFFF` | Kernel Dynamic Heap & VMalloc | Ring 0 (U/S=0, NX) | Expandable kernel heap, DMA/MMIO, kernel stacks with guard pages |
| `0xFFFF_FFFF_8000_0000` – `0xFFFF_FFFF_FFFF_FFFF` | Kernel ELF Image (`.text`, `.rodata`, `.data`, `.bss`) | Ring 0 (U/S=0) | Higher-half `-2 GiB` (`code-model=kernel`); `.text` RX, `.rodata` R, `.data`/`.bss` RW+NX |

## Security Invariants

1. **Address Space Isolation**: Each user process owns a distinct PML4 whose upper half (indices 256..511) mirrors kernel mappings (with `U/S=0`), and whose lower half (indices 0..255) is private to that process.
2. **User Pointer Boundary**: Any pointer `>= 0x0000_8000_0000_0000` passed to a syscall is unconditionally rejected with `EFAULT`.
3. **W^X & NX**: EFER.NXE is enabled during early boot; writable data/stack/heap pages always set the NX bit.
4. **SMEP & SMAP**: Probed via CPUID and enabled when supported by the CPU.

## Consequences

- Kernel remains mapped during syscalls and interrupts without switching CR3 on entry (KPTI may be evaluated in a future hardening phase).
- Physical frame translation via `phys + hhdm_offset` is O(1) and clean for page-table walks.
