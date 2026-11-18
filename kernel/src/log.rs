//! Structured kernel logging subsystem and `dmesg` ring buffer.

use crate::arch::x86_64::serial;
use crate::drivers::fbcon;
use crate::sync::IrqSpinLock;
use core::fmt;

const DMESG_BUFFER_SIZE: usize = 64 * 1024; // 64 KiB circular kernel log buffer

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
}

impl LogLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN ",
            Self::Info => "INFO ",
            Self::Debug => "DEBUG",
        }
    }
}

struct DmesgRing {
    buf: [u8; DMESG_BUFFER_SIZE],
    write_pos: usize,
    total_written: usize,
}

impl DmesgRing {
    const fn new() -> Self {
        Self {
            buf: [0; DMESG_BUFFER_SIZE],
            write_pos: 0,
            total_written: 0,
        }
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.buf[self.write_pos] = b;
            self.write_pos = (self.write_pos + 1) % DMESG_BUFFER_SIZE;
            self.total_written = self.total_written.saturating_add(1);
        }
    }

    /// Copy up to `out.len()` bytes of the most recent kernel log into `out`.
    pub fn snapshot(&self, out: &mut [u8]) -> usize {
        let len = self.total_written.min(DMESG_BUFFER_SIZE).min(out.len());
        if self.total_written <= DMESG_BUFFER_SIZE {
            out[..len].copy_from_slice(&self.buf[..len]);
        } else {
            let start = (self.write_pos + DMESG_BUFFER_SIZE - len) % DMESG_BUFFER_SIZE;
            for (i, slot) in out.iter_mut().enumerate().take(len) {
                *slot = self.buf[(start + i) % DMESG_BUFFER_SIZE];
            }
        }
        len
    }
}

impl fmt::Write for DmesgRing {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_bytes(s.as_bytes());
        Ok(())
    }
}

static DMESG: IrqSpinLock<DmesgRing> = IrqSpinLock::new(DmesgRing::new());

struct DualWriter;

impl fmt::Write for DualWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        serial::write_str(s);
        DMESG.lock().push_bytes(s.as_bytes());
        fbcon::write_str(s);
        Ok(())
    }
}

static LOGGER_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

pub fn log_fmt(level: LogLevel, args: fmt::Arguments<'_>) {
    let _guard = LOGGER_LOCK.lock();
    let mut w = DualWriter;
    let _ = fmt::Write::write_fmt(
        &mut w,
        format_args!("[AetherOS {}] {}\n", level.as_str(), args),
    );
}

pub fn read_dmesg(out: &mut [u8]) -> usize {
    DMESG.lock().snapshot(out)
}

#[macro_export]
macro_rules! kinfo {
    ($($arg:tt)*) => {
        $crate::log::log_fmt($crate::log::LogLevel::Info, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! kwarn {
    ($($arg:tt)*) => {
        $crate::log::log_fmt($crate::log::LogLevel::Warn, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! kerror {
    ($($arg:tt)*) => {
        $crate::log::log_fmt($crate::log::LogLevel::Error, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! kdebug {
    ($($arg:tt)*) => {
        $crate::log::log_fmt($crate::log::LogLevel::Debug, format_args!($($arg)*))
    };
}
