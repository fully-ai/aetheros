# Changelog

## [v0.1.0-alpha.1] - 2026-11-18

### Added
- **Milestone 0 & 1 (Boot & x86_64 Kernel Foundation)**: Limine Boot Protocol v3 UEFI + Legacy BIOS hybrid GPT boot, COM1 16550 UART, linear framebuffer console, structured `dmesg` logger, RBP panic backtrace, CPUID feature detection (`CR0.WP`, `EFER.NXE`, `EFER.SCE`, `CR4.PGE`, `CR4.SMEP`), 64-bit GDT + TSS (`IST1..3`), 256-entry IDT, 8259 PIC remap (`32..47`), and fail-closed Bitmap PMM.
- **Milestone 2 (Memory Management)**: 4-level `x86_64` page tables, `AddressSpace` VMM with `W^X` enforcement, Demand Paging (`PTE_DEMAND`), Copy-on-Write (`PTE_COW`) + PMM `SharedRefTable` reference counting, Guard Pages (`PTE_GUARD`), and 2 MiB Coalescing Kernel Heap Allocator (`0xAE74_EC01` canary).
- **Milestone 3 (Scheduler & Synchronization)**: 1000 Hz PIT (`IRQ0`) preemptive priority round-robin scheduler, `SpinLock`, `IrqSpinLock`, `WaitQueue`, `Mutex`, `Semaphore`, `RwLock`.
- **Milestone 4 (Ring 3 & Syscall ABI v1)**: Per-CPU `GS_BASE`/`KERNEL_GS_BASE`, `SYSCALL`/`SYSRET` entry (`IA32_LSTAR`), canonical user pointer validation (`mm::user_access`), `fork`, `wait4`, `brk`, `mmap`, `munmap`, `mprotect`, and Ring 3 `#PF`/`#GP` process fault isolation.
- **Milestone 5 (IPC)**: Anonymous pipes (`pipe`/`pipe2`), named FIFOs (`mkfifo`), `AF_UNIX` stream/dgram sockets, shared memory (`shm_open`/`shm_unlink` + `mmap`), signals (`kill`), and `poll`.
- **Milestone 6 (VFS)**: Unified VFS, Linux-like directory hierarchy, `/dev/{null,zero,random,urandom,console,tty,serial}`, `/proc/{version,uptime,meminfo,cpuinfo,mounts,syscalls,gui}`, `/tmp` sticky bit, symlinks/hardlinks, and USTAR `initramfs.tar` unpacker.
- **Milestone 7 (AetherFS v1 `>= 1 PiB`)**: 64-bit extent-based journaling filesystem (`crates/aether-fs-core`), Primary (`LBA 1`) + Backup (`LBA 2`) superblock failover, Castagnoli CRC32C checksums on superblocks/inodes/journal, and Write-Ahead Journal (`AETHJRNL`) crash recovery.
- **Milestone 8, 9, 10, 11 (ELF64 & Ring 3 User Space)**: Fuzz-tested ELF64 loader (`crates/aether-elf` + `kernel/src/sched/exec.rs`), UUID identity database (`root`, `aether`, `wheel`), UUID ACLs, `execute_as_uuid` transitions, compiled Ring 3 `/sbin/init`, `/bin/aethersh` (pipelines `|`, redirections `>`/`>>`/`<`), `/bin/sudo`, `/bin/aetherpkg`, and 31 core utilities (`ls`, `cat`, `grep`, `wc`, `ps`, `kill`, `id`, `whoami`, `groups`, `stat`, `chmod`, `chown`, `chgrp`, `getfacl`, `setfacl`, `useradd`, etc.).
- **Milestone 12, 13, 14 (Drivers, Net, Hardening & GUI)**: PCI Type-1 bus scanner, CMOS RTC, PS/2 controller, Ethernet/ARP/IPv4/ICMP/UDP network stack (2,000-packet fuzz tested), `RDTSC` hardware microbenchmarks, and `800x500` double-buffered 2D desktop compositor (`kernel/src/gui/mod.rs`).
