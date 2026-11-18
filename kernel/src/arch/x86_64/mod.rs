//! x86_64 architecture-specific hardware primitives, CPU initialization,
//! segmentation (GDT/TSS), interrupts (IDT/PIC), and serial console.

pub mod cpu;
pub mod gdt;
pub mod idt;
pub mod pic;
pub mod serial;

use core::arch::asm;

/// Read an 8-bit value from an x86 I/O port.
#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    unsafe {
        asm!("in al, dx", in("dx") port, out("al") val, options(nomem, nostack, preserves_flags));
    }
    val
}

/// Write an 8-bit value to an x86 I/O port.
#[inline]
pub unsafe fn outb(port: u16, val: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
    }
}

/// Read a 16-bit value from an x86 I/O port.
#[inline]
pub unsafe fn inw(port: u16) -> u16 {
    let val: u16;
    unsafe {
        asm!("in ax, dx", in("dx") port, out("ax") val, options(nomem, nostack, preserves_flags));
    }
    val
}

/// Write a 16-bit value to an x86 I/O port.
#[inline]
pub unsafe fn outw(port: u16, val: u16) {
    unsafe {
        asm!("out dx, ax", in("dx") port, in("ax") val, options(nomem, nostack, preserves_flags));
    }
}

/// Read a 32-bit value from an x86 I/O port.
#[inline]
pub unsafe fn inl(port: u16) -> u32 {
    let val: u32;
    unsafe {
        asm!("in eax, dx", in("dx") port, out("eax") val, options(nomem, nostack, preserves_flags));
    }
    val
}

/// Write a 32-bit value to an x86 I/O port.
#[inline]
pub unsafe fn outl(port: u16, val: u32) {
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") val, options(nomem, nostack, preserves_flags));
    }
}

/// Brief I/O delay via unused port `0x80`.
#[inline]
pub unsafe fn io_wait() {
    unsafe { outb(0x80, 0) };
}

/// Disable maskable hardware interrupts on the local CPU (`cli`).
#[inline]
pub fn cli() {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }
}

/// Enable maskable hardware interrupts on the local CPU (`sti`).
#[inline]
pub fn sti() {
    unsafe {
        asm!("sti", options(nomem, nostack, preserves_flags));
    }
}

/// Halt the CPU until the next interrupt (`hlt`).
#[inline]
pub fn hlt() {
    unsafe {
        asm!("hlt", options(nomem, nostack, preserves_flags));
    }
}

/// Read the current `RFLAGS` register.
#[inline]
pub fn read_rflags() -> u64 {
    let rflags: u64;
    unsafe {
        asm!("pushfq; pop {}", out(reg) rflags, options(nomem, preserves_flags));
    }
    rflags
}

/// Returns `true` if `RFLAGS.IF` (bit 9) is set.
#[inline]
pub fn are_interrupts_enabled() -> bool {
    (read_rflags() & (1 << 9)) != 0
}

/// Permanently halt the current CPU with interrupts disabled.
pub fn halt_loop() -> ! {
    cli();
    loop {
        hlt();
    }
}

/// Exit QEMU via the `isa-debug-exit` device at I/O port `0xf4` (used ONLY in automated test mode).
/// QEMU exits with status `((code as u32) << 1) | 1`.
pub fn qemu_debug_exit(code: u32) -> ! {
    unsafe {
        outl(0xf4, code);
    }
    halt_loop()
}
