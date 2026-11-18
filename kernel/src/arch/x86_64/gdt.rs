//! x86_64 Global Descriptor Table (GDT) and Task State Segment (TSS).
//!
//! Selector layout is intentionally ordered for x86_64 `SYSCALL`/`SYSRET`:
//! - `0x00`: Null descriptor
//! - `0x08`: Kernel Code 64-bit (`DPL=0`)  -> `STAR[47:32] = 0x08` (`CS=0x08`, `SS=0x10`)
//! - `0x10`: Kernel Data        (`DPL=0`)
//! - `0x18`: User Compat slot   (unused)   -> `STAR[63:48] = 0x18` (`SS=0x23`, `CS=0x2B`)
//! - `0x20`: User Data          (`DPL=3`, selector `0x23`)
//! - `0x28`: User Code 64-bit   (`DPL=3`, selector `0x2B`)
//! - `0x30`: 64-bit TSS         (16 bytes, occupies `0x30..=0x3F`)

use core::arch::asm;
use core::mem::size_of;
use core::ptr::addr_of;

pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_DS: u16 = 0x10;
pub const USER_BASE_STAR: u16 = 0x18;
pub const USER_DS: u16 = 0x20 | 3; // 0x23
pub const USER_CS: u16 = 0x28 | 3; // 0x2B
pub const TSS_SELECTOR: u16 = 0x30;

/// IST index (1-based in IDT gate descriptor) for Double Fault (`#DF`).
pub const IST_DOUBLE_FAULT: u8 = 1;
/// IST index for Non-Maskable Interrupt (`#NMI`).
pub const IST_NMI: u8 = 2;
/// IST index for Machine Check (`#MC`).
pub const IST_MACHINE_CHECK: u8 = 3;

const IST_STACK_SIZE: usize = 16 * 1024;

#[repr(C, align(16))]
struct AlignedStack([u8; IST_STACK_SIZE]);

static mut DOUBLE_FAULT_STACK: AlignedStack = AlignedStack([0; IST_STACK_SIZE]);
static mut NMI_STACK: AlignedStack = AlignedStack([0; IST_STACK_SIZE]);
static mut MC_STACK: AlignedStack = AlignedStack([0; IST_STACK_SIZE]);
static mut INITIAL_KERNEL_RSP0_STACK: AlignedStack = AlignedStack([0; IST_STACK_SIZE]);

#[repr(C, packed)]
pub struct TaskStateSegment {
    reserved0: u32,
    pub rsp: [u64; 3],
    reserved1: u64,
    pub ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    pub iomap_base: u16,
}

const _: () = assert!(size_of::<TaskStateSegment>() == 104);

#[repr(C, align(16))]
struct GdtTable {
    entries: [u64; 8], // 6 standard descriptors + 2 slots for 16-byte 64-bit TSS
}

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

static mut TSS: TaskStateSegment = TaskStateSegment {
    reserved0: 0,
    rsp: [0; 3],
    reserved1: 0,
    ist: [0; 7],
    reserved2: 0,
    reserved3: 0,
    iomap_base: size_of::<TaskStateSegment>() as u16,
};

static mut GDT: GdtTable = GdtTable {
    entries: [
        0x0000_0000_0000_0000, // 0x00: Null
        0x00AF_9A00_0000_FFFF, // 0x08: Kernel 64-bit Code (P=1, DPL=0, S=1, Exec/Read, L=1)
        0x00CF_9200_0000_FFFF, // 0x10: Kernel Data (P=1, DPL=0, S=1, Read/Write)
        0x0000_0000_0000_0000, // 0x18: User Compat placeholder for STAR
        0x00CF_F200_0000_FFFF, // 0x20: User Data (P=1, DPL=3, S=1, Read/Write)
        0x00AF_FA00_0000_FFFF, // 0x28: User 64-bit Code (P=1, DPL=3, S=1, Exec/Read, L=1)
        0x0000_0000_0000_0000, // 0x30: TSS low (filled in init)
        0x0000_0000_0000_0000, // 0x38: TSS high (filled in init)
    ],
};

fn stack_top(stack: *const AlignedStack) -> u64 {
    let base = stack as usize;
    ((base + IST_STACK_SIZE) & !0xF) as u64
}

/// Initialize the GDT, populate IST stacks in the TSS, load GDTR, reload CS/DS/SS/ES/FS/GS,
/// and load the Task Register (`ltr`).
pub fn init() {
    unsafe {
        // Populate TSS stacks
        TSS.rsp[0] = stack_top(addr_of!(INITIAL_KERNEL_RSP0_STACK));
        TSS.ist[0] = stack_top(addr_of!(DOUBLE_FAULT_STACK)); // IST1
        TSS.ist[1] = stack_top(addr_of!(NMI_STACK)); // IST2
        TSS.ist[2] = stack_top(addr_of!(MC_STACK)); // IST3
        TSS.iomap_base = size_of::<TaskStateSegment>() as u16;

        // Encode 64-bit TSS descriptor at GDT entries [6] and [7] (selector 0x30)
        let tss_base = addr_of!(TSS) as u64;
        let tss_limit = (size_of::<TaskStateSegment>() - 1) as u64;
        let low: u64 = (tss_limit & 0xFFFF)
            | ((tss_base & 0x00FF_FFFF) << 16)
            | (0x89u64 << 40) // P=1, DPL=0, S=0, Type=0x9 (Available 64-bit TSS)
            | (((tss_limit >> 16) & 0xF) << 48)
            | (((tss_base >> 24) & 0xFF) << 56);
        let high: u64 = tss_base >> 32;

        GDT.entries[6] = low;
        GDT.entries[7] = high;

        let gdtr = DescriptorTablePointer {
            limit: (size_of::<GdtTable>() - 1) as u16,
            base: addr_of!(GDT) as u64,
        };

        // Load GDTR, far-return to reload CS=0x08, reload data segments to 0x10, and load TSS=0x30
        asm!(
            "lgdt [{gdtr}]",
            "push {kcs}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            "mov ds, {kds:x}",
            "mov es, {kds:x}",
            "mov ss, {kds:x}",
            "xor {tmp:e}, {tmp:e}",
            "mov fs, {tmp:x}",
            "mov gs, {tmp:x}",
            "ltr {tss:x}",
            gdtr = in(reg) &gdtr,
            kcs = const KERNEL_CS as u64,
            kds = in(reg) KERNEL_DS as u64,
            tss = in(reg) TSS_SELECTOR as u64,
            tmp = lateout(reg) _,
            options(preserves_flags)
        );
    }
}

/// Update `TSS.rsp0` so hardware transitions from Ring 3 land on the active thread's kernel stack.
pub fn set_kernel_stack(rsp0: u64) {
    unsafe {
        TSS.rsp[0] = rsp0;
    }
}

/// Read current `CS` register (used for diagnostics and self-tests).
pub fn read_cs() -> u16 {
    let cs: u16;
    unsafe { asm!("mov {:x}, cs", out(reg) cs, options(nomem, nostack, preserves_flags)) };
    cs
}

/// Read current Task Register (`str`, used for diagnostics and self-tests).
pub fn read_tr() -> u16 {
    let tr: u16;
    unsafe { asm!("str {:x}", out(reg) tr, options(nomem, nostack, preserves_flags)) };
    tr
}
