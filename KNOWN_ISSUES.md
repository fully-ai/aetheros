# AetherOS Known Limitations & Architectural Scope Notes

Per Project Rule 0.2, all current scope boundaries of `v0.1.0-alpha.1` are explicitly documented below:

1. **SMP Application Processor (AP) Scheduling**:
   - Limine enumerates all host vCPUs (`CPUs: 2 (BSP LAPIC=0)`), and the scheduler runs preemptively on the Bootstrap Processor (BSP) at 1000 Hz (`IRQ0`). Dedicated per-AP runqueues and AP trampoline wakeup are scheduled for `v0.2.0`.
2. **Static ELF64 User-Space Binaries**:
   - The ELF64 loader (`crates/aether-elf` + `kernel/src/sched/exec.rs`) supports statically linked `ET_EXEC` and fixed-base `ET_DYN` `x86_64` binaries (`0x400000+`). Dynamic shared library loading (`PT_INTERP` / `ld-aether.so`) is not yet implemented.
3. **Storage & Network Hardware Transport**:
   - The live `/mnt/aether` AetherFS v1 volume runs against an in-memory block device (and sparse 2 PiB block backend in `aetherfs_1pib_and_crash_recovery`), while PCI bus enumeration detects QEMU's VirtIO/IDE/E1000 controllers. Binding VirtIO-Blk and VirtIO-Net PCI DMA ring queues is planned for `v0.2.0`.
4. **Development-Mode `sudo`**:
   - In QEMU development builds, `/etc/sudoers` enables `%wheel ALL=(ALL) NOPASSWD_DEV: ALL` via `SYS_ELEVATE_SUDO`, which prints a mandatory warning (`[sudo] WARNING: QEMU development passwordless mode active`) and records an immutable `[AUDIT]` entry on every elevation.
