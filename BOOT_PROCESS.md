# AetherOS Boot Process Architecture

See also [KERNEL_DESIGN.md](file:///home/ubeyt/Belgeler/AetherOS/KERNEL_DESIGN.md) and [docs/adr/0001-bootloader-limine.md](file:///home/ubeyt/Belgeler/AetherOS/docs/adr/0001-bootloader-limine.md).

## 1. Hybrid UEFI + Legacy BIOS GPT Disk Image (`scripts/image.sh`)

Every build produces a 64 MiB GPT disk image (`build/aetheros.img`) containing a 62 MiB FAT32 EFI System Partition (`ESP`) and embedded Limine v9 BIOS stages:

```text
build/aetheros.img (64 MiB GPT Hybrid Disk)
├── LBA 0..2047        : Protective MBR + Primary GPT Header + Limine BIOS Stage 2
└── Partition 1 (ESP)  : FAT32 (LBA 2048..129023)
    ├── /EFI/BOOT/BOOTX64.EFI     # Limine x86_64 UEFI bootloader
    ├── /boot/limine/limine.conf  # Boot menu & module configuration
    ├── /boot/limine/limine-bios.sys
    ├── /boot/aetheros.elf        # Higher-half x86_64 kernel ELF (-2 GiB)
    └── /boot/initramfs.tar       # USTAR archive with Ring 3 ELF64 user-space binaries & /etc
```

## 2. Limine Boot Protocol v3 Handoff (`kernel/src/boot/mod.rs`)
- Requests `BaseRevision::new(3)`, `HhdmRequest`, `KernelAddressRequest`, `MemoryMapRequest`, `MpRequest`, `FramebufferRequest`, `ExecutableCmdlineRequest`, and `ModuleRequest` (`/boot/initramfs.tar`).
- Transitions directly into `kernel_main` (`_start`) in 64-bit Long Mode with identity/HHDM mappings active at `0xffff800000000000` and kernel sections mapped at `0xffffffff80000000`.
