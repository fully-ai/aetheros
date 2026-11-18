//! Per-CPU Kernel State accessed via `GS` (`IA32_GS_BASE` in Ring 0, swapped via `swapgs`).
//!
//! Layout of `PerCpuData` is ABI-contracted with assembly in `syscall_entry.rs`:
//! - Offset `0`: `kernel_rsp` (top of active task's kernel stack)
//! - Offset `8`: `user_rsp_scratch` (temporary scratch slot for user `RSP` during `SYSCALL`)
//! - Offset `16`: `cpu_id`

use crate::arch::x86_64::cpu::{wrmsr, MSR_GS_BASE, MSR_KERNEL_GS_BASE};
use core::ptr::addr_of_mut;

#[repr(C, align(64))]
pub struct PerCpuData {
    pub kernel_rsp: u64,
    pub user_rsp_scratch: u64,
    pub cpu_id: u32,
    pub _pad: u32,
}

static mut BSP_PER_CPU: PerCpuData = PerCpuData {
    kernel_rsp: 0,
    user_rsp_scratch: 0,
    cpu_id: 0,
    _pad: 0,
};

/// Initialize `IA32_GS_BASE` to point to `BSP_PER_CPU` in Ring 0 and clear `IA32_KERNEL_GS_BASE`.
/// Must be called AFTER `gdt::init()`.
pub fn init() {
    unsafe {
        let ptr = addr_of_mut!(BSP_PER_CPU) as u64;
        wrmsr(MSR_GS_BASE, ptr);
        wrmsr(MSR_KERNEL_GS_BASE, 0);
    }
}

/// Update the kernel stack top (`gs:[0]`) used by `SYSCALL` entry from Ring 3.
#[inline]
pub fn set_kernel_stack(rsp0: u64) {
    unsafe {
        BSP_PER_CPU.kernel_rsp = rsp0;
    }
}

#[inline]
pub fn get_kernel_stack() -> u64 {
    unsafe { BSP_PER_CPU.kernel_rsp }
}
