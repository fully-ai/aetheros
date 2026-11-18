# AetherOS Kernel Design

## 1. Boot Sequence & Higher-Half Initialization (`kernel/src/main.rs`)

The AetherOS kernel boots via the **Limine Boot Protocol v3** (`limine` crate v0.5) on both **UEFI (OVMF)** and **Legacy BIOS** targets. Upon entry to `_start()` in `kernel/src/main.rs` (running in 64-bit Long Mode at `0xffffffff80000000`), initialization proceeds through 14 deterministic stages:

1. **Stage 1 — Early Serial UART**: Initializes `COM1` (`0x3F8`, 16550 UART, 115200 8N1) for immediate structured diagnostic output and panic backtraces.
2. **Stage 2 — Bootloader Handoff (`boot::acquire_boot_info`)**: Acquires the HHDM offset (`0xffff800000000000`), kernel physical/virtual addresses, memory map regions, SMP CPU descriptors, framebuffer geometry, kernel command line, and the `initramfs.tar` module.
3. **Stage 3 — Framebuffer Console (`drivers::fbcon::init`)**: Initializes the linear 32-bit RGB/BGR framebuffer text console with an embedded 8x8 glyph renderer.
4. **Stage 4 — CPU Feature Detection & Hardware Protections (`arch::x86_64::cpu`)**: Queries `CPUID` leaf `0x1`, `0x7`, and `0x80000001`, and enforces:
   - `CR0.WP` (bit 16): Supervisor cannot overwrite read-only pages.
   - `EFER.NXE` (bit 11): Enables the page-table No-Execute (`NX`, bit 63) protection bit.
   - `EFER.SCE` (bit 0): Enables fast `SYSCALL`/`SYSRET` instructions.
   - `CR4.PGE` (bit 7): Enables Global TLB entries for kernel pages.
   - `CR4.SMEP` (bit 20): Prevents Ring 0 from executing instructions mapped on `PTE_USER` pages.
5. **Stage 5 — GDT & 64-Bit TSS (`arch::x86_64::gdt`)**: Loads a 7-entry Global Descriptor Table with `SYSCALL`/`SYSRET`-ordered segment selectors (`KERNEL_CS=0x08`, `KERNEL_DS=0x10`, `USER_DS=0x23`, `USER_CS=0x2B`, and a 16-byte 64-bit `TSS` descriptor at `0x30`). Configures dedicated 16 KiB Interrupt Stack Table stacks for `IST1` (`#DF` Double Fault), `IST2` (`#NMI`), and `IST3` (`#MC`).
6. **Stage 6 — 256-Entry IDT & 8259 PIC (`arch::x86_64::idt`, `pic`)**: Installs assembly stubs (`isr_stub_0..255`) for all 32 CPU exceptions and 224 interrupt vectors, and remaps the dual 8259 PICs to vectors `32..47` (`0x20..0x2F`) to eliminate overlap with CPU exceptions.
7. **Stage 7 — Physical Memory Manager (`mm::pmm`)**: Initializes the fail-closed bitmap frame allocator and `SharedRefTable` Copy-on-Write reference counter.
8. **Stage 8 — Virtual Memory Manager & Kernel Heap (`mm::vmm`, `mm::heap`)**: Installs the `#PF` page-fault hook (handling Demand Paging `PTE_DEMAND`, Copy-on-Write `PTE_COW`, and Guard Pages `PTE_GUARD`) and maps the 2 MiB coalescing kernel heap (`0xffffc00000000000`).
9. **Stage 9 — Per-CPU & Fast `SYSCALL` Entry (`arch::x86_64::percpu`, `syscall_entry`)**: Programs `IA32_GS_BASE`, `IA32_KERNEL_GS_BASE`, `IA32_STAR`, `IA32_LSTAR` (`syscall_entry_stub`), and `IA32_FMASK`.
10. **Stage 10 — Security, VFS, `initramfs.tar` & AetherFS v1 (`security`, `fs`)**: Initializes the UUID identity database, mounts root VFS, unpacks `/boot/initramfs.tar`, and mounts a live AetherFS v1 volume on `/mnt/aether`.
11. **Stage 11 — Device Drivers & Network Stack (`drivers::pci`, `rtc`, `net`)**: Enumerates the PCI bus, reads CMOS RTC UTC time, and brings up `lo` (`127.0.0.1/8`) and `eth0` (`10.0.2.15/24`).
12. **Stage 12 — Preemptive Scheduler & 1000 Hz PIT (`sched`, `timer`)**: Spawns `kthreadd/boot` and `idle/0`, configures the 1000 Hz PIT (`IRQ0 -> vector 32`), and enables hardware interrupts (`sti`).
13. **Stage 13 — Self-Test Suite (`selftest::maybe_run`)**: Executes automated Milestone 1–14 tests when `selftest=all` is present on the kernel command line.
14. **Stage 14 — Desktop Compositor & Ring 3 `/sbin/init`**: Renders the initial 3-window desktop workspace and spawns `/sbin/init` (PID 1) in Ring 3.

---

## 2. Exception Isolation & Panic Diagnostics (`kernel/src/panic.rs`)

- **User-Space Fault Containment**: When a CPU exception (`#PF`, `#GP`, `#UD`, `#DE`, `#SS`) occurs with `TrapFrame::from_user_mode() == true` (`(cs & 3) == 3`), the kernel logs the diagnostic trap state and invokes `sched::terminate_faulting_user_task(frame, vec)`, terminating **only** the offending Ring 3 process with exit status `128 + signal` (`139` for `SIGSEGV`) and immediately context-switching to the next runnable task.
- **Kernel Panic Handler**: If an unrecoverable fault occurs in Ring 0, `panic.rs` disables interrupts, dumps the fault location and register state, and walks the `%rbp` frame pointer chain (`MAX_FRAMES = 16`) with higher-half canonical pointer validation to emit a complete stack backtrace over `COM1`.

---

## 3. Kernel Synchronization Primitives (`kernel/src/sync/`)

| Primitive | File | Semantics |
| :--- | :--- | :--- |
| `SpinLock<T>` | `sync/mod.rs` | Atomic test-and-test-and-set (`AtomicBool`) spinlock with `pause` backoff for short critical sections that never cross IRQ handlers |
| `IrqSpinLock<T>` | `sync/mod.rs` | Saves `RFLAGS`, disables local interrupts (`cli`), acquires lock, and restores previous `IF` state on drop; required for PMM, scheduler, VFS, and serial locks |
| `WaitQueue` | `sync/waitqueue.rs` | FIFO queue of blocked task IDs (`tid`) supporting `prepare_to_wait`, `sleep_Current`, `wake_one`, and `wake_all` |
| `Mutex<T>` | `sync/mutex.rs` | Sleeping mutual exclusion lock backed by an owner TID and a `WaitQueue` |
| `Semaphore` | `sync/semaphore.rs` | Counting sleeping semaphore (`down` / `up`) backed by a `WaitQueue` |
| `RwLock<T>` | `sync/mod.rs` | Reader-writer spinlock (`AtomicUsize` readers + exclusive writer bit) |
