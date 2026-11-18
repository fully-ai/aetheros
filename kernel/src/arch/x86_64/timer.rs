//! System Timer Subsystem (8254 PIT at 1000 Hz + TSC & Local APIC detection).
//! Drives the preemptive scheduler on IRQ0 (IDT vector 32) and maintains monotonic uptime.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::x86_64::cpu::{rdmsr, MSR_APIC_BASE};
use crate::arch::x86_64::idt::{self, TrapFrame};
use crate::arch::x86_64::{outb, pic};

pub const PIT_FREQUENCY_HZ: u64 = 1000;
const PIT_BASE_HZ: u64 = 1_193_182;
const PIT_CMD_PORT: u16 = 0x43;
const PIT_CH0_PORT: u16 = 0x40;

pub const TIMER_IRQ: u8 = 0;
pub const TIMER_VECTOR: u8 = pic::PIC1_OFFSET + TIMER_IRQ; // Vector 32

static TICKS_MS: AtomicU64 = AtomicU64::new(0);
static APIC_PHYS_BASE: AtomicU64 = AtomicU64::new(0);

/// Returns monotonic ticks since boot (1 tick = 1 millisecond at 1000 Hz).
#[inline]
pub fn ticks() -> u64 {
    TICKS_MS.load(Ordering::Relaxed)
}

/// Returns monotonic uptime in milliseconds.
#[inline]
pub fn uptime_ms() -> u64 {
    ticks()
}

/// Returns monotonic uptime in nanoseconds.
#[inline]
pub fn uptime_ns() -> u64 {
    ticks().saturating_mul(1_000_000)
}

/// Returns the detected Local APIC physical base address from `IA32_APIC_BASE`.
#[inline]
pub fn lapic_phys_base() -> u64 {
    APIC_PHYS_BASE.load(Ordering::Relaxed)
}

fn on_timer_irq(frame: &mut TrapFrame) -> *mut TrapFrame {
    let now = TICKS_MS.fetch_add(1, Ordering::Relaxed) + 1;
    // Acknowledge legacy 8259 PIC IRQ0 BEFORE invoking scheduler so the next tick can fire
    // even if `iretq` switches into a brand-new thread.
    pic::send_eoi(TIMER_IRQ);
    crate::sched::on_timer_tick(frame, now)
}

/// Initialize the 8254 PIT to 1000 Hz (1 ms period), record Local APIC base MSR,
/// register the timer interrupt vector (32), and unmask IRQ0 on the 8259 PIC.
pub fn init() {
    let apic_msr = unsafe { rdmsr(MSR_APIC_BASE) };
    APIC_PHYS_BASE.store(apic_msr & 0x000F_FFFF_FFFF_F000, Ordering::Relaxed);

    let divisor = (PIT_BASE_HZ / PIT_FREQUENCY_HZ) as u16;
    unsafe {
        // Channel 0, lobyte/hibyte, rate generator (mode 2), binary
        outb(PIT_CMD_PORT, 0x34);
        outb(PIT_CH0_PORT, (divisor & 0xFF) as u8);
        outb(PIT_CH0_PORT, (divisor >> 8) as u8);
    }

    idt::register_handler(TIMER_VECTOR, on_timer_irq);
    pic::unmask_irq(TIMER_IRQ);
}
