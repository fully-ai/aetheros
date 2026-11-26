# AetherOS Performance & Benchmark Report (`PERFORMANCE.md`)

See also [BENCHMARKS.md](BENCHMARKS.md).

## 1. Hardware `RDTSC` & Compositor Performance Metrics

| Subsystem / Operation | Measured Performance | Optimization Strategy |
| :--- | :--- | :--- |
| **PMM 4 KiB Frame Alloc + Free (`alloc_frame` + `free_frame`)** | **`~1,636 – 1,809` cycles/op** | Rolling `next_hint` cursor + 8-frame (`0xFF` byte) bitmap fast skip |
| **Kernel Heap Alloc + Free (256-byte `Vec<u64>`)** | **`~762 – 974` cycles/op** | 8 MiB pre-mapped initial arena + immediate adjacent free-block coalescing |
| **VFS Virtual File Open + Read (`/proc/version`)** | **`~8,725 – 10,159` cycles/op** | Canonical path normalization + in-memory `BTreeMap` lookup |
| **2D Desktop Compositor (`1280x800` BGRX Framebuffer Blit)** | **`~60 FPS` (`16 ms` tick, dirty-flag driven)** | Direct row-wise `copy_nonoverlapping` for 32-bit BGRX linear framebuffers (`drivers::fbcon::blit_rgb_buffer`) + dirty-frame suppression when idle |
| **Multi-Catalog Binary Packaging (`initramfs.tar`)** | **`2.0 MiB` image (65 commands)** | Shared multi-call `/bin/ls` ELF64 binary + VFS symlinks (`insert_symlink`) eliminating 3.5 MiB of duplicate ELF allocations |
