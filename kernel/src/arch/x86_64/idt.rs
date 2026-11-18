//! x86_64 Interrupt Descriptor Table (IDT), assembly trap stubs, and exception/IRQ dispatch.

use core::arch::{asm, global_asm};
use core::mem::size_of;
use core::ptr::addr_of;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::x86_64::{cpu, gdt, pic};

/// Full CPU register snapshot saved on the kernel stack by `isr_common_stub`.
/// Field order MUST exactly match the `push`/`pop` sequence in `isr_common_stub`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error_code: u64,
    // Hardware-pushed iretq frame:
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl TrapFrame {
    #[inline]
    pub fn from_user_mode(&self) -> bool {
        (self.cs & 0x3) == 0x3
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            zero: 0,
        }
    }

    fn set_handler(&mut self, handler_addr: u64, ist_index: u8, dpl: u8, is_trap: bool) {
        self.offset_low = handler_addr as u16;
        self.selector = gdt::KERNEL_CS;
        self.ist = ist_index & 0x7;
        let gate_type: u8 = if is_trap { 0x0F } else { 0x0E }; // 64-bit Trap Gate vs Interrupt Gate
        self.type_attr = 0x80 | ((dpl & 0x3) << 5) | gate_type;
        self.offset_mid = (handler_addr >> 16) as u16;
        self.offset_high = (handler_addr >> 32) as u32;
        self.zero = 0;
    }
}

#[repr(C, align(16))]
struct IdtTable([IdtEntry; 256]);

#[repr(C, packed)]
struct Idtr {
    limit: u16,
    base: u64,
}

static mut IDT: IdtTable = IdtTable([IdtEntry::missing(); 256]);

/// Function pointer type for registered IRQ or custom exception callbacks.
/// Returns the `TrapFrame` pointer to restore (normally the same `frame`, or a new task's frame).
pub type IrqHandler = fn(&mut TrapFrame) -> *mut TrapFrame;

static mut IRQ_HANDLERS: [Option<IrqHandler>; 256] = [None; 256];

/// Count of `#BP` (breakpoint) traps observed — used by Milestone 1 self-test.
static BREAKPOINT_COUNT: AtomicU64 = AtomicU64::new(0);
/// Count of `#PF` (page fault) traps observed.
static PAGE_FAULT_COUNT: AtomicU64 = AtomicU64::new(0);
/// Optional user-fault termination hook installed once process manager is active (Milestone 4+).
static mut USER_FAULT_HOOK: Option<fn(&mut TrapFrame) -> *mut TrapFrame> = None;
/// Optional page-fault VMM hook installed in Milestone 2+.
static mut PAGE_FAULT_HOOK: Option<fn(&mut TrapFrame, u64) -> Option<*mut TrapFrame>> = None;

pub fn breakpoint_count() -> u64 {
    BREAKPOINT_COUNT.load(Ordering::Relaxed)
}

pub fn page_fault_count() -> u64 {
    PAGE_FAULT_COUNT.load(Ordering::Relaxed)
}

pub fn register_handler(vector: u8, handler: IrqHandler) {
    unsafe {
        IRQ_HANDLERS[vector as usize] = Some(handler);
    }
}

pub fn register_page_fault_hook(hook: fn(&mut TrapFrame, u64) -> Option<*mut TrapFrame>) {
    unsafe {
        PAGE_FAULT_HOOK = Some(hook);
    }
}

pub fn register_user_fault_hook(hook: fn(&mut TrapFrame) -> *mut TrapFrame) {
    unsafe {
        USER_FAULT_HOOK = Some(hook);
    }
}

extern "C" {
    static isr_stub_base: u8;
}

const ISR_STUB_STRIDE: u64 = 16;

// Generate all 256 interrupt/exception entry stubs at a fixed 16-byte stride
// starting at `isr_stub_base`, followed by the common register save/restore trampoline.
// Stack layout upon entering `isr_common_stub`:
//   [rsp + 0]  = vector
//   [rsp + 8]  = error_code (hardware or synthetic 0)
//   [rsp + 16] = rip
//   [rsp + 24] = cs
//   [rsp + 32] = rflags
//   [rsp + 40] = rsp
//   [rsp + 48] = ss
global_asm!(
    r#"
.section .text

.balign 16
.global isr_stub_base
isr_stub_base:
.set vec_num, 0
.rept 256
    .balign 16
    .if (vec_num == 8) || (vec_num == 10) || (vec_num == 11) || (vec_num == 12) || (vec_num == 13) || (vec_num == 14) || (vec_num == 17) || (vec_num == 21) || (vec_num == 29) || (vec_num == 30)
        nop
        nop
    .else
        push 0
    .endif
    push vec_num
    jmp isr_common_stub
    .set vec_num, vec_num + 1
.endr

.balign 16
.global isr_common_stub
isr_common_stub:
    // If interrupted from Ring 3 (CS & 3 != 0), swap GS to kernel per-CPU GS
    test qword ptr [rsp + 24], 3
    jz 1f
    swapgs
1:
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    cld
    mov rdi, rsp
    call isr_dispatch_rust
    mov rsp, rax

    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax

    // Skip vector + error_code (16 bytes)
    add rsp, 16

    // If returning to Ring 3 (now CS is at [rsp + 8]), swap GS back to user GS
    test qword ptr [rsp + 8], 3
    jz 2f
    swapgs
2:
    iretq
"#
);

fn exception_name(vector: u64) -> &'static str {
    match vector {
        0 => "#DE Divide-by-Zero",
        1 => "#DB Debug",
        2 => "#NMI Non-Maskable Interrupt",
        3 => "#BP Breakpoint",
        4 => "#OF Overflow",
        5 => "#BR Bound Range Exceeded",
        6 => "#UD Invalid Opcode",
        7 => "#NM Device Not Available",
        8 => "#DF Double Fault",
        9 => "Coprocessor Segment Overrun",
        10 => "#TS Invalid TSS",
        11 => "#NP Segment Not Present",
        12 => "#SS Stack-Segment Fault",
        13 => "#GP General Protection Fault",
        14 => "#PF Page Fault",
        16 => "#MF x87 Floating-Point Exception",
        17 => "#AC Alignment Check",
        18 => "#MC Machine Check",
        19 => "#XM SIMD Floating-Point Exception",
        20 => "#VE Virtualization Exception",
        21 => "#CP Control Protection Exception",
        29 => "#VC VMM Communication Exception",
        30 => "#SX Security Exception",
        _ => "Unknown/Reserved Exception",
    }
}

#[no_mangle]
extern "C" fn isr_dispatch_rust(frame: &mut TrapFrame) -> *mut TrapFrame {
    let vec = frame.vector;

    // 1. Registered IRQ / custom vector handler (vectors >= 32)
    if (32..256).contains(&vec) {
        if let Some(handler) = unsafe { IRQ_HANDLERS[vec as usize] } {
            return handler(frame);
        }
        // Unhandled legacy PIC IRQ? Acknowledge PIC cleanly to avoid wedging the bus.
        if (32..48).contains(&vec) {
            pic::send_eoi((vec - 32) as u8);
        }
        return frame as *mut TrapFrame;
    }

    // 2. Breakpoint (#BP, vector 3): resumable diagnostic trap
    if vec == 3 {
        BREAKPOINT_COUNT.fetch_add(1, Ordering::Relaxed);
        crate::kdebug!(
            "IDT: #BP breakpoint caught at RIP={:#018x} CS={:#x} RSP={:#018x}",
            frame.rip,
            frame.cs,
            frame.rsp
        );
        return frame as *mut TrapFrame;
    }

    // 3. Page Fault (#PF, vector 14): read CR2 and delegate to VMM hook if registered
    if vec == 14 {
        PAGE_FAULT_COUNT.fetch_add(1, Ordering::Relaxed);
        let cr2 = cpu::read_cr2();
        if let Some(hook) = unsafe { PAGE_FAULT_HOOK } {
            if let Some(next_frame) = hook(frame, cr2) {
                return next_frame;
            }
        }
        if frame.from_user_mode() {
            crate::kwarn!(
                "USER #PF: addr={:#018x} rip={:#018x} err={:#x} (P={} W={} U={} R={} I={})",
                cr2,
                frame.rip,
                frame.error_code,
                frame.error_code & 1,
                (frame.error_code >> 1) & 1,
                (frame.error_code >> 2) & 1,
                (frame.error_code >> 3) & 1,
                (frame.error_code >> 4) & 1,
            );
            if let Some(hook) = unsafe { USER_FAULT_HOOK } {
                return hook(frame);
            }
        }
        panic!(
            "Unhandled Kernel Page Fault (#PF): fault_addr={:#018x} err={:#x} (P={} W={} U={} R={} I={})\n  RIP={:#018x} CS={:#x} RFLAGS={:#018x} RSP={:#018x} SS={:#x}\n  RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
            cr2,
            frame.error_code,
            frame.error_code & 1,
            (frame.error_code >> 1) & 1,
            (frame.error_code >> 2) & 1,
            (frame.error_code >> 3) & 1,
            (frame.error_code >> 4) & 1,
            frame.rip,
            frame.cs,
            frame.rflags,
            frame.rsp,
            frame.ss,
            frame.rax,
            frame.rbx,
            frame.rcx,
            frame.rdx,
        );
    }

    // 4. Fatal user-mode CPU exceptions must terminate ONLY the offending user process (Section 3.3)
    if frame.from_user_mode() {
        crate::kwarn!(
            "USER EXCEPTION {} (vec={}) err={:#x} RIP={:#018x} RSP={:#018x}",
            exception_name(vec),
            vec,
            frame.error_code,
            frame.rip,
            frame.rsp
        );
        if let Some(hook) = unsafe { USER_FAULT_HOOK } {
            return hook(frame);
        }
    }

    // 5. Fatal kernel-mode exception -> detailed panic + backtrace
    panic!(
        "Fatal Kernel Exception: {} (vector={}) error_code={:#x}\n  RIP={:#018x} CS={:#x} RFLAGS={:#018x} RSP={:#018x} SS={:#x}\n  RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}\n  RSI={:#018x} RDI={:#018x} RBP={:#018x} CR2={:#018x}",
        exception_name(vec),
        vec,
        frame.error_code,
        frame.rip,
        frame.cs,
        frame.rflags,
        frame.rsp,
        frame.ss,
        frame.rax,
        frame.rbx,
        frame.rcx,
        frame.rdx,
        frame.rsi,
        frame.rdi,
        frame.rbp,
        cpu::read_cr2(),
    );
}

/// Populate all 256 IDT entries (assigning IST stacks to `#DF`, `#NMI`, `#MC`) and execute `lidt`.
pub fn init() {
    unsafe {
        let base = addr_of!(isr_stub_base) as u64;
        let entries_ptr = (&raw mut IDT.0) as *mut IdtEntry;
        for vec in 0..256usize {
            let stub_addr = base + (vec as u64) * ISR_STUB_STRIDE;
            let ist = match vec {
                2 => gdt::IST_NMI,
                8 => gdt::IST_DOUBLE_FAULT,
                18 => gdt::IST_MACHINE_CHECK,
                _ => 0,
            };
            // Allow Ring 3 software breakpoint (#BP = 3) for debugging; all others DPL=0.
            let dpl = if vec == 3 { 3 } else { 0 };
            let mut entry = IdtEntry::missing();
            entry.set_handler(stub_addr, ist, dpl, false);
            entries_ptr.add(vec).write(entry);
        }

        let idtr = Idtr {
            limit: (size_of::<IdtTable>() - 1) as u16,
            base: addr_of!(IDT) as u64,
        };

        asm!("lidt [{}]", in(reg) &idtr, options(readonly, nostack, preserves_flags));
    }
}
