//! Legacy 8259A Programmable Interrupt Controller (PIC) driver.
//! Remaps hardware IRQs 0..15 to IDT vectors 32..47 so they never collide with
//! architectural CPU exceptions (vectors 0..31).

use crate::arch::x86_64::{inb, io_wait, outb};

const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

const ICW1_ICW4: u8 = 0x01;
const ICW1_INIT: u8 = 0x10;
const ICW4_8086: u8 = 0x01;
const PIC_EOI: u8 = 0x20;

pub const PIC1_OFFSET: u8 = 32;
pub const PIC2_OFFSET: u8 = 40;

/// Remap both 8259 PICs to vectors 32..47 and mask all IRQs initially.
pub fn init_and_mask_all() {
    unsafe {
        // Start initialization sequence (cascade mode)
        outb(PIC1_CMD, ICW1_INIT | ICW1_ICW4);
        io_wait();
        outb(PIC2_CMD, ICW1_INIT | ICW1_ICW4);
        io_wait();

        // ICW2: vector offsets
        outb(PIC1_DATA, PIC1_OFFSET);
        io_wait();
        outb(PIC2_DATA, PIC2_OFFSET);
        io_wait();

        // ICW3: master has slave on IRQ2 (bit 2), slave cascade identity = 2
        outb(PIC1_DATA, 0x04);
        io_wait();
        outb(PIC2_DATA, 0x02);
        io_wait();

        // ICW4: 8086/88 mode
        outb(PIC1_DATA, ICW4_8086);
        io_wait();
        outb(PIC2_DATA, ICW4_8086);
        io_wait();

        // Mask all IRQs until explicitly unmasked by drivers / APIC setup
        outb(PIC1_DATA, 0xFF);
        outb(PIC2_DATA, 0xFF);
    }
}

pub fn unmask_irq(irq: u8) {
    unsafe {
        if irq < 8 {
            let mask = inb(PIC1_DATA) & !(1 << irq);
            outb(PIC1_DATA, mask);
        } else if irq < 16 {
            // Also unmask cascade line IRQ2 on master
            let m1 = inb(PIC1_DATA) & !(1 << 2);
            outb(PIC1_DATA, m1);
            let mask = inb(PIC2_DATA) & !(1 << (irq - 8));
            outb(PIC2_DATA, mask);
        }
    }
}

pub fn mask_irq(irq: u8) {
    unsafe {
        if irq < 8 {
            let mask = inb(PIC1_DATA) | (1 << irq);
            outb(PIC1_DATA, mask);
        } else if irq < 16 {
            let mask = inb(PIC2_DATA) | (1 << (irq - 8));
            outb(PIC2_DATA, mask);
        }
    }
}

pub fn send_eoi(irq: u8) {
    unsafe {
        if irq >= 8 {
            outb(PIC2_CMD, PIC_EOI);
        }
        outb(PIC1_CMD, PIC_EOI);
    }
}
