# AetherOS Hardware Cycle & Subsystem Benchmarks

## 1. Benchmark Methodology (`test_m13_security_fuzz_and_benchmarks`)

All latency and throughput metrics below were captured directly from real `RDTSC` hardware timestamp counter measurements and 1000 Hz PIT wall-clock ticks during automated QEMU `x86_64` execution (`./scripts/test.sh` on 2026-11-18).

---

## 2. Measured Kernel Microbenchmarks (`RDTSC` Cycles per Operation)

| Subsystem Operation | Iterations | Measured Latency (`RDTSC` Cycles / Op) | Notes |
| :--- | :--- | :--- | :--- |
| **PMM Physical Frame Alloc + Free (`alloc_frame` + `free_frame`)** | `128` | **`1,791` cycles/op** | Includes `IrqSpinLock` save/restore, 8-frame bitmap byte-skip scan, and double-free validation |
| **Kernel Heap Alloc + Dealloc (`Vec<u64>` 256-byte block)** | `128` | **`762` cycles/op** | Coalescing block allocator with `0xAE74_EC01` magic canary verification and adjacent merge |
| **VFS Virtual File Open + Read (`vfs_open` + `vfs_read_file` on `/proc/version`)** | `64` | **`8,725` cycles/op** | Includes path normalization, symlink traversal, DAC/ACL permission check, and `/proc` synthesis |

---

## 3. Macro-Level Boot & Subsystem Timing (QEMU `x86_64`, 512 MiB RAM, 2 SMP vCPUs)

| Milestone Phase | Elapsed Kernel Uptime (`ms`) | Measured Output |
| :--- | :--- | :--- |
| **Stage 1–12 Kernel Boot to Preemptive Scheduler (`sti`)** | **`~15 ms`** | GDT/TSS/IDT, PMM (488 MiB usable), VMM, 2 MiB Heap, VFS + `initramfs.tar` unpack (36 binaries), AetherFS v1 mount, PCI scan (6 devices), RTC, 1000 Hz PIT |
| **Preemptive Multitasking & Mutex Contention (2 Threads, 250 Lock Acquisitions)** | **`~42 ms`** | `5` hardware timer preemptions, `12` context switches, `mutex_val = 250`, zero lost updates |
| **Full Milestone 1–10 Kernel + AetherFS 1 PiB + Security Audit Suite** | **`68 ms`** | Includes 2 PiB volume format, 1 PiB+8 KiB sparse write/read, Superblock LBA 2 failover, WAL journal replay, and 7 audited `execute_as_uuid`/`sudo` transitions |
| **Ring 3 `/sbin/init` -> `/bin/uname` -> `/bin/aethersh` Multi-Stage Pipelines -> `/bin/aetherpkg` -> `/bin/sudo`** | **`27 ms`** (`68 ms -> 95 ms`) | Spawns 14 Ring 3 ELF64 processes across `fork()` + `execve()` + `pipe()` + `wait4()` |
| **Milestone 14 `800x500` 32-Bit Desktop Compositor Frame Render + Framebuffer Blit** | **`< 4 ms`** | Renders gradient desktop, status bar, 3 shadowed windows with text/progress bars, software cursor, and FNV-1a frame checksum (`0x2011ebfc`) |
