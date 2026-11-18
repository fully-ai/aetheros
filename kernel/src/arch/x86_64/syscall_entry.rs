//! x86_64 `SYSCALL` / `SYSRET` MSR initialization and assembly entry trampoline.
//!
//! On `SYSCALL` from Ring 3:
//! - CPU saves user `RIP` in `RCX`, user `RFLAGS` in `R11`, masks `RFLAGS` with `IA32_FMASK`,
//!   loads `CS = 0x08` (`KERNEL_CS`), `SS = 0x10` (`KERNEL_DS`), and jumps to `syscall_entry_stub`.
//! - `syscall_entry_stub` executes `swapgs`, stashes user `RSP` in `gs:[8]`, loads the current
//!   task's kernel stack top from `gs:[0]`, builds a complete `TrapFrame`, and calls
//!   `syscall_dispatch_from_trapframe(&mut TrapFrame) -> *mut TrapFrame`.
//! - Returns through `isr_restore_trapframe` (which executes `swapgs` when returning to Ring 3).

use core::arch::global_asm;

use crate::arch::x86_64::cpu::{wrmsr, MSR_FMASK_BITS, MSR_LSTAR, MSR_SFMASK, MSR_STAR};
use crate::arch::x86_64::gdt::{KERNEL_CS, USER_BASE_STAR};
use crate::arch::x86_64::idt::TrapFrame;

extern "C" {
    fn syscall_entry_stub();
}

global_asm!(
    r#"
.section .text
.balign 16
.global syscall_entry_stub
syscall_entry_stub:
    swapgs
    mov gs:[8], rsp
    mov rsp, gs:[0]

    // Construct a full hardware-compatible TrapFrame on the kernel stack
    push 0x23               // ss  = USER_DS (0x20 | 3)
    push qword ptr gs:[8]   // rsp = saved user RSP
    push r11                // rflags = saved user RFLAGS (in R11)
    push 0x2B               // cs  = USER_CS (0x28 | 3)
    push rcx                // rip = saved user RIP (in RCX)
    push 0                  // error_code = 0
    push 0x80               // vector = 0x80 (SYSCALL)

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
    call syscall_dispatch_from_trapframe
    mov rsp, rax
    jmp isr_restore_trapframe
"#
);

#[no_mangle]
extern "C" fn syscall_dispatch_from_trapframe(frame: &mut TrapFrame) -> *mut TrapFrame {
    crate::syscall::dispatch(frame)
}

/// Program `IA32_STAR`, `IA32_LSTAR`, and `IA32_FMASK` MSRs for 64-bit `SYSCALL`.
pub fn init() {
    let star: u64 = ((USER_BASE_STAR as u64) << 48) | ((KERNEL_CS as u64) << 32);
    let lstar: u64 = syscall_entry_stub as *const () as usize as u64;
    unsafe {
        wrmsr(MSR_STAR, star);
        wrmsr(MSR_LSTAR, lstar);
        wrmsr(MSR_SFMASK, MSR_FMASK_BITS);
    }
}
