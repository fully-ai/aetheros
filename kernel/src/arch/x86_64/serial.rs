//! 16550 UART serial console driver (COM1 at I/O port 0x3F8).

use crate::arch::x86_64::{inb, outb};
use crate::sync::IrqSpinLock;
use core::fmt;

pub const COM1_PORT: u16 = 0x3F8;

pub struct SerialPort {
    base: u16,
    initialized: bool,
}

impl SerialPort {
    pub const fn new(base: u16) -> Self {
        Self {
            base,
            initialized: false,
        }
    }

    /// Initialize the 16550 UART to 115200 baud, 8N1, FIFO enabled, with loopback check.
    pub fn init(&mut self) -> bool {
        unsafe {
            outb(self.base + 1, 0x00); // Disable all interrupts
            outb(self.base + 3, 0x80); // Enable DLAB (set baud rate divisor)
            outb(self.base, 0x01); // Divisor low byte: 115200 baud
            outb(self.base + 1, 0x00); // Divisor high byte
            outb(self.base + 3, 0x03); // 8 bits, no parity, one stop bit
            outb(self.base + 2, 0xC7); // Enable FIFO, clear them, 14-byte threshold
            outb(self.base + 4, 0x0B); // IRQs enabled, RTS/DSR set

            // Loopback self-test
            outb(self.base + 4, 0x1E); // Set in loopback mode, test serial chip
            outb(self.base, 0xAE); // Send test byte
            if inb(self.base) != 0xAE {
                self.initialized = false;
                return false;
            }

            // Normal operation mode (not-loopback, IRQs enabled, OUT1/OUT2 set)
            outb(self.base + 4, 0x0F);
        }
        self.initialized = true;
        true
    }

    #[inline]
    fn is_transmit_empty(&self) -> bool {
        unsafe { (inb(self.base + 5) & 0x20) != 0 }
    }

    #[inline]
    pub fn has_data(&self) -> bool {
        unsafe { (inb(self.base + 5) & 0x01) != 0 }
    }

    pub fn write_byte(&self, byte: u8) {
        if !self.initialized {
            return;
        }
        if byte == b'\n' {
            self.write_raw(b'\r');
        }
        self.write_raw(byte);
    }

    #[inline]
    fn write_raw(&self, byte: u8) {
        // Bounded spin to avoid hanging forever on headless hardware without UART
        for _ in 0..100_000 {
            if self.is_transmit_empty() {
                unsafe { outb(self.base, byte) };
                return;
            }
            core::hint::spin_loop();
        }
    }

    pub fn read_byte_nonblocking(&self) -> Option<u8> {
        if !self.initialized || !self.has_data() {
            return None;
        }
        Some(unsafe { inb(self.base) })
    }

    pub fn enable_rx_interrupt(&self) {
        if self.initialized {
            unsafe {
                outb(self.base + 1, 0x01); // Data Available interrupt
            }
        }
    }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            self.write_byte(b);
        }
        Ok(())
    }
}

pub static COM1: IrqSpinLock<SerialPort> = IrqSpinLock::new(SerialPort::new(COM1_PORT));

pub fn init() -> bool {
    COM1.lock().init()
}

pub fn write_str(s: &str) {
    let mut port = COM1.lock();
    let _ = fmt::Write::write_str(&mut *port, s);
}

pub fn write_fmt(args: fmt::Arguments<'_>) {
    let mut port = COM1.lock();
    let _ = fmt::Write::write_fmt(&mut *port, args);
}

/// Unlocked emergency write for fatal kernel panics.
///
/// # Safety
/// Only call during a fatal kernel panic after disabling interrupts.
pub unsafe fn emergency_write_fmt(args: fmt::Arguments<'_>) {
    unsafe { COM1.force_unlock() };
    let mut port = COM1.lock();
    let _ = fmt::Write::write_fmt(&mut *port, args);
}

pub fn read_byte_nonblocking() -> Option<u8> {
    COM1.lock().read_byte_nonblocking()
}
