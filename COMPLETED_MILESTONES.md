# Completed Milestones Log

1. **Milestone 0 — Architecture & Repository Setup**:
   - Authored ADRs `0001` through `0006` in `docs/adr/`.
   - Created shared `no_std` crates `crates/aether-abi` and `crates/aether-uuid`.
   - Created deterministic build, image packaging, QEMU run/test/debug, formatting, and linting scripts in `scripts/*.sh`.
2. **Milestone 1 — Bootable x86_64 Kernel Foundation**:
   - Implemented Limine Boot Protocol v3 handoff (`kernel/src/boot/mod.rs`), COM1 16550 UART serial console, linear framebuffer console, structured `dmesg` ring buffer, RBP panic backtrace, CPUID feature detection & hardware protections (`CR0.WP`, `EFER.NXE`, `EFER.SCE`, `CR4.PGE`, `CR4.SMEP`), 64-bit GDT+TSS with `IST1..3` stacks, 256-entry IDT, 8259 PIC remap (`32..47`), and fail-closed Bitmap PMM.
3. **Milestone 2 — Memory Management**:
   - Implemented 4-level `x86_64` page tables (`kernel/src/arch/x86_64/paging.rs`) and `AddressSpace` VMM (`kernel/src/mm/vmm.rs`) with `W^X` enforcement, Demand Paging (`PTE_DEMAND`), Copy-on-Write (`PTE_COW`) + PMM `SharedRefTable` reference counting, Guard Pages (`PTE_GUARD`), and the 2 MiB Coalescing Kernel Heap Allocator (`kernel/src/mm/heap.rs`).
4. **Milestone 3 — Interrupts, Synchronization & Preemptive Scheduler**:
   - Implemented 1000 Hz PIT timer (`IRQ0 -> vector 32`), `SpinLock`, `IrqSpinLock`, `WaitQueue`, `Mutex`, `Semaphore`, `RwLock`, and the priority round-robin preemptive scheduler (`kernel/src/sched/mod.rs`).
5. **Milestone 4 — Ring 3 User Mode & Syscall ABI v1**:
   - Implemented Per-CPU `GS_BASE`/`KERNEL_GS_BASE` and `SYSCALL`/`SYSRET` entry (`kernel/src/arch/x86_64/syscall_entry.rs`), strict user pointer validation (`kernel/src/mm/user_access.rs`), `sys_fork`, `sys_wait4`, `sys_brk`, `sys_mmap`, `sys_munmap`, `sys_mprotect`, and Ring 3 `#PF`/`#GP` fault isolation.
6. **Milestone 5 — Inter-Process Communication (IPC)**:
   - Implemented Anonymous Pipes (`pipe`/`pipe2`), Named FIFOs (`mkfifo`), Unix-Domain stream & datagram sockets (`AF_UNIX`), Shared Memory objects (`shm_open`/`shm_unlink` + `mmap`), signals (`kill`), and `poll`.
7. **Milestone 6 — Virtual Filesystem (VFS)**:
   - Implemented unified VFS (`kernel/src/fs/mod.rs`), Linux-like directory hierarchy, `/dev/{null,zero,random,urandom,console,tty,serial}`, `/proc/{version,uptime,meminfo,cpuinfo,mounts,syscalls,gui}`, `/tmp` sticky-bit enforcement, symlinks/hardlinks, and USTAR `initramfs.tar` unpacker.
8. **Milestone 7 — AetherFS v1 (`>= 1 PiB`)**:
   - Implemented `crates/aether-fs-core` and `kernel/src/fs/aetherfs.rs` with 64-bit block addressing (`>= 1 PiB` capacity verified on a 2 PiB volume with sparse file I/O at `1 PiB + 8 KiB`), Primary (`LBA 1`) and Backup (`LBA 2`) superblocks with automatic failover, Castagnoli CRC32C checksums on superblocks/inodes/journal, and Write-Ahead Journal (`AETHJRNL`) crash recovery.
9. **Milestone 8 — ELF64 Program Loader**:
   - Implemented `crates/aether-elf` (10,000-mutation fuzz tested) and `kernel/src/sched/exec.rs` (`PT_LOAD` `W^X` mapping + guarded 32 KiB Ring 3 stack with `argv`, `envp`, `auxv`).
10. **Milestone 9 — Init Process, Native Shell (`aethersh`) & Core Utilities**:
    - Built Ring 3 ELF64 user-space workspace (`userspace/`) containing `libaether`, `/sbin/init`, `/bin/aethersh` (pipelines `|`, redirections `>`/`>>`/`<`, quoting, `$VAR`), and 28 core utilities (`/bin/ls`, `/bin/cat`, `/bin/grep`, `/bin/wc`, `/bin/ps`, `/bin/id`, `/bin/stat`, `/bin/chmod`, `/bin/chown`, `/bin/df`, `/bin/dmesg`, etc.).
11. **Milestone 10 — Multi-User UUID Identity, ACLs, `execute_as_uuid` & `sudo`**:
    - Implemented UUID identity database (`root` UID 0, `aether` UID 1000, `wheel` GID 10), per-inode UUID ACL evaluation, root-controlled `execute_as_uuid` transitions (`MS_NOSUID` & `no_new_privs` enforced), and audited `/bin/sudo` elevation.
12. **Milestone 11 — Native Package Manager (`aetherpkg`)**:
    - Implemented `/bin/aetherpkg` with recursive dependency resolution (`aether-devtools -> aether-base`), Castagnoli CRC32C payload verification, and `/var/lib/aetherpkg/installed.db`.
13. **Milestone 12 — Device Drivers & Network Stack**:
    - Implemented PCI Bus Type-1 enumeration (`kernel/src/drivers/pci.rs`), CMOS RTC UTC clock & PS/2 keyboard polling (`kernel/src/drivers/rtc.rs`), and Ethernet/ARP/IPv4/ICMP/UDP network stack (`kernel/src/net/mod.rs`, fuzz-tested against 2,000 mutated packets).
14. **Milestone 13 — Security Hardening, Fuzzing & Benchmarks**:
    - Implemented adversarial user pointer boundary fuzzing, VFS tamper verification, and `RDTSC` hardware cycle benchmarks (`test_m13_security_fuzz_and_benchmarks`).
15. **Milestone 14 — Framebuffer 2D Desktop Compositor & Window Manager**:
    - Implemented `kernel/src/gui/mod.rs` double-buffered `800x500` 2D software compositor, 3 z-ordered system windows, mouse/keyboard input routing, hardware framebuffer blitting, and `/proc/gui` reporting.
