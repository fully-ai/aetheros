# AetherOS Engineering Roadmap & Milestone Status

## 1. Completed Milestones (v0.1.0-alpha.1)

| Milestone | Title | Status | Verification |
| :--- | :--- | :--- | :--- |
| **Milestone 0** | Architecture, Repository Structure & ADRs `0001..0006` | **COMPLETED** | `scripts/lint.sh`, `docs/adr/` |
| **Milestone 1** | Bootable x86_64 Kernel Foundation (Limine v3 UEFI+BIOS, GDT/TSS/IST, IDT, PIC, Bitmap PMM) | **COMPLETED** | `boot_handoff`, `cpu_protections`, `gdt_and_tss`, `idt_breakpoint_exception`, `pmm_allocator` |
| **Milestone 2** | Virtual Memory Manager (`W^X`, Demand Paging, COW) & Coalescing Kernel Heap | **COMPLETED** | `vmm_page_mapping_and_wx`, `vmm_demand_paging`, `vmm_address_space_isolation_and_cow`, `kernel_heap_and_alloc` |
| **Milestone 3** | 1000 Hz Timer, Synchronization (`Mutex`, `Semaphore`, `WaitQueue`) & Preemptive Scheduler | **COMPLETED** | `timer_and_sleep`, `preemptive_multitasking_and_sync` |
| **Milestone 4** | Ring 3 User Mode, `SYSCALL` ABI v1, User Pointer Validation, `fork`/`wait4` & Fault Isolation | **COMPLETED** | `ring3_syscalls_fork_and_pointer_validation`, `ring3_fault_isolation` |
| **Milestone 5** | IPC Architecture (Anonymous Pipes, Named FIFOs, `AF_UNIX` Sockets, Shared Memory) | **COMPLETED** | `ipc_pipes_unix_sockets_and_shm` |
| **Milestone 6** | Virtual Filesystem (VFS), FHS Hierarchy, `/dev`, `/proc`, `/tmp` Sticky Bit & `initramfs.tar` | **COMPLETED** | `vfs_hierarchy_devices_and_proc` |
| **Milestone 7** | AetherFS v1 (`>= 1 PiB` 64-Bit Extents, Dual Superblocks, CRC32C, WAL Journal Recovery) | **COMPLETED** | `aetherfs_1pib_and_crash_recovery` + 4 host `aether-fs-core` unit tests |
| **Milestone 8** | ELF64 Loader & Initial Ring 3 Stack (`argv`, `envp`, `auxv`, 10,000-Mutation Fuzz Test) | **COMPLETED** | `elf64_loader_and_ring3_execution` + 4 host `aether-elf` unit/fuzz tests |
| **Milestone 9** | Ring 3 `/sbin/init`, `/bin/aethersh` (Pipelines, Redirections) & 28 Core Utilities | **COMPLETED** | `userspace_init_aethersh_pipelines_and_aetherpkg` |
| **Milestone 10** | Multi-User UUID Identity Model, UUID ACLs, `execute_as_uuid` & Audited `/bin/sudo` | **COMPLETED** | `multiuser_permissions_acl_and_execute_as` |
| **Milestone 11** | Native Package Manager (`/bin/aetherpkg` Dependency Resolution & CRC32C Verification) | **COMPLETED** | `userspace_init_aethersh_pipelines_and_aetherpkg` |
| **Milestone 12** | Device Drivers (PCI Bus, CMOS RTC, PS/2) & Network Stack (Ethernet/ARP/IPv4/ICMP/UDP) | **COMPLETED** | `drivers_pci_rtc_and_network_stack` |
| **Milestone 13** | Security Hardening, Boundary Fuzzing & Hardware `RDTSC` Microbenchmarks | **COMPLETED** | `m13_security_fuzz_and_benchmarks` |
| **Milestone 14** | Framebuffer 2D Desktop Compositor, Window Manager & Input Routing | **COMPLETED** | `m14_framebuffer_compositor_and_gui` |

---

## 2. Future Roadmap (v0.2.0+)

1. **SMP Multi-Core AP Scheduling**: Bring up Application Processors (APs) via Limine MP wakeup with per-core runqueues and IPI TLB shootdowns.
2. **VirtIO-Blk & VirtIO-Net PCI DMA Drivers**: Bind the PCI scanner's detected VirtIO devices (`0x1AF4`) to ring-buffer DMA queues for persistent disk block I/O and external SLIRP TCP/IP traffic.
3. **Dynamic ELF64 Shared Library Loader (`ld-aether.so`)**: Support `PT_INTERP` and `ET_DYN` position-independent shared libraries under `/lib`.
4. **Ring 3 IPC Window Protocol**: Expose the Milestone 14 desktop compositor via `/run/compositor.sock` (`AF_UNIX`) + `shm_open` surfaces so user-space GUI applications can render client windows directly from Ring 3.
