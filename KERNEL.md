# AetherOS Kernel Specification

See also [KERNEL_DESIGN.md](KERNEL_DESIGN.md) and [ARCHITECTURE.md](ARCHITECTURE.md).

## 1. Kernel Execution & Memory Model

- **Target**: `x86_64-unknown-none` (`# ![no_std]`, `# ![no_main]`).
- **Virtual Base**: Higher-half `-2 GiB` (`0xffffffff80000000`) with Higher-Half Direct Map (`HHDM`) at `0xffff800000000000`.
- **Privilege Separation**:
  - **Ring 0 (`CS=0x08`, `SS=0x10`)**: Trusted kernel execution context.
  - **Ring 3 (`CS=0x2B`, `SS=0x23`)**: Unprivileged user-space execution context with isolated per-process `CR3` (`PML4[0..256]`).
- **Hardware Enforcements (`arch::x86_64::cpu`)**:
  - `CR0.WP = 1`: Ring 0 cannot write to read-only pages (enforcing Copy-on-Write even during kernel `copy_to_user`).
  - `EFER.NXE = 1`: Hardware No-Execute bit (`bit 63`) enforced on all data, heap, and stack pages (`W^X`).
  - `EFER.SCE = 1`: Fast `SYSCALL`/`SYSRET` privilege transition instructions enabled.
  - `CR4.PGE = 1`: Global TLB pages enabled for higher-half kernel mappings.
  - `CR4.SMEP = 1`: Supervisor Mode Execution Prevention blocks Ring 0 from executing code on `PTE_USER` pages.

---

## 2. Kernel Boot & Subsystem Initialization Sequence

```mermaid
sequenceDiagram
    participant FW as UEFI (OVMF) / BIOS
    participant LIM as Limine v9 Bootloader
    participant KMAIN as kernel_main (_start)
    participant CPU as GDT/TSS/IDT/PIC/PMM/VMM
    participant FS as VFS / initramfs / AetherFS v1
    participant SCHED as 1000 Hz Preemptive Scheduler
    participant INIT as /sbin/init (Ring 3 PID 1)

    FW->>LIM: Load GPT ESP / MBR Stage 2
    LIM->>KMAIN: Handoff in 64-bit Long Mode (HHDM + aetheros.elf + initramfs.tar)
    KMAIN->>CPU: Stage 1-9: Serial COM1, FBCon, CPUID/WP/NX/SMEP, GDT+TSS(IST1..3), IDT(256), PIC, PMM, VMM, 2MiB Heap, SYSCALL MSRs
    KMAIN->>FS: Stage 10-11: UUID Identity DB, VFS, unpack initramfs.tar, mount AetherFS v1 (/mnt/aether), PCI, RTC, Net
    KMAIN->>SCHED: Stage 12-13: Spawn kthreadd/boot + idle/0, start 1000 Hz PIT (IRQ0), run selftests (if cmdline selftest=all)
    KMAIN->>INIT: Stage 14: Render 2D Desktop Compositor & spawn /sbin/init in Ring 3
```
