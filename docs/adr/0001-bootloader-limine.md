# ADR-0001: Bootloader Selection — Limine v9 (UEFI + BIOS Hybrid)

- **Status**: Accepted
- **Date**: 2026-11-18
- **Decision Makers**: AetherOS Core Engineering

## Context

AetherOS targets x86_64/AMD64 and needs a reliable, modern 64-bit boot handoff for QEMU (both UEFI via OVMF and legacy BIOS) and eventually physical hardware. Key requirements:
- Direct entry in 64-bit Long Mode with 4-level paging enabled.
- Higher-half kernel loading (`0xffffffff80000000`).
- Higher-Half Direct Map (HHDM) for safe physical memory access.
- Normalized physical memory map (E820 / UEFI memory map unified).
- Boot module loading (for initial user-space archive `initramfs.tar`).
- ACPI RSDP, framebuffer, and MP/SMP handoff without requiring custom 16-bit real-mode/32-bit protected-mode trampoline bloat.
- Reproducible disk image creation without requiring root privileges or loop mounts.

## Decision

Adopt the **Limine Boot Protocol (Base Revision 3)** using pinned Limine `v9.x-binary` (`ee5d29cd0a8034612dcd1df3f00052480db785c5`).

Boot images are assembled as hybrid GPT FAT32 disk images (`build/aetheros.img`) containing both `/EFI/BOOT/BOOTX64.EFI` (UEFI x86_64) and `/boot/limine/limine-bios.sys` installed via `limine bios-install` (Legacy BIOS). The kernel implements the 0BSD-licensed Limine protocol structures directly in `kernel/src/boot/limine.rs` with zero external crate dependencies.

## Alternatives Considered

1. **GRUB2 / Multiboot2**: Enters in 32-bit protected mode without paging, requires hand-written 32->64 bootstrap stubs, and relies heavily on `grub-mkrescue` + `xorriso`.
2. **Custom UEFI Bootloader**: High maintenance cost for early kernel milestones; delays core kernel engineering.
3. **Rust `bootloader` crate**: Tightly coupled to nightly features and opinionated Cargo integration.

## Consequences

- Kernel begins directly in 64-bit higher-half virtual memory with HHDM and a clean memory map.
- Both UEFI (OVMF) and BIOS QEMU boot paths work from a single disk image.
- Image assembly uses standard user-space tools (`dd`, `parted`, `mkfs.fat`, `mcopy`, `limine`).
