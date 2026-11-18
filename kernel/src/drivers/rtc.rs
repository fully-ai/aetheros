//! CMOS Real-Time Clock (RTC) and PS/2 Keyboard Input Drivers.

use crate::arch::x86_64::{inb, outb};

const CMOS_ADDR: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

const PS2_DATA: u16 = 0x60;
const PS2_STATUS: u16 = 0x64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtcDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

#[inline]
fn read_cmos(reg: u8) -> u8 {
    unsafe {
        outb(CMOS_ADDR, reg);
        inb(CMOS_DATA)
    }
}

#[inline]
const fn bcd_to_bin(val: u8) -> u8 {
    (val & 0x0F) + ((val >> 4) * 10)
}

/// Read the current UTC date and time from the CMOS Real-Time Clock.
pub fn read_rtc_datetime() -> RtcDateTime {
    // Wait lightly if update-in-progress bit is set
    for _ in 0..1000 {
        if (read_cmos(0x0A) & 0x80) == 0 {
            break;
        }
        core::hint::spin_loop();
    }
    let mut sec = read_cmos(0x00);
    let mut min = read_cmos(0x02);
    let mut hr = read_cmos(0x04);
    let mut day = read_cmos(0x07);
    let mut mon = read_cmos(0x08);
    let mut yr = read_cmos(0x09);
    let reg_b = read_cmos(0x0B);

    if (reg_b & 0x04) == 0 {
        sec = bcd_to_bin(sec);
        min = bcd_to_bin(min);
        hr = bcd_to_bin(hr & 0x7F);
        day = bcd_to_bin(day);
        mon = bcd_to_bin(mon);
        yr = bcd_to_bin(yr);
    }

    RtcDateTime {
        year: 2000 + (yr as u16),
        month: mon.clamp(1, 12),
        day: day.clamp(1, 31),
        hour: hr % 24,
        minute: min % 60,
        second: sec % 60,
    }
}

/// Non-blocking poll of the 8042 PS/2 keyboard controller output buffer, translating
/// Set 1 make-scancodes into ASCII bytes.
pub fn poll_ps2_ascii() -> Option<u8> {
    let status = unsafe { inb(PS2_STATUS) };
    if (status & 0x01) == 0 {
        return None;
    }
    let scancode = unsafe { inb(PS2_DATA) };
    if (scancode & 0x80) != 0 {
        return None; // Key release break code
    }
    match scancode {
        0x02..=0x0A => Some(b'1' + (scancode - 0x02)),
        0x0B => Some(b'0'),
        0x1C => Some(b'\n'),
        0x39 => Some(b' '),
        0x10 => Some(b'q'),
        0x11 => Some(b'w'),
        0x12 => Some(b'e'),
        0x13 => Some(b'r'),
        0x14 => Some(b't'),
        0x15 => Some(b'y'),
        0x16 => Some(b'u'),
        0x17 => Some(b'i'),
        0x18 => Some(b'o'),
        0x19 => Some(b'p'),
        0x1E => Some(b'a'),
        0x1F => Some(b's'),
        0x20 => Some(b'd'),
        0x21 => Some(b'f'),
        0x22 => Some(b'g'),
        0x23 => Some(b'h'),
        0x24 => Some(b'j'),
        0x25 => Some(b'k'),
        0x26 => Some(b'l'),
        0x2C => Some(b'z'),
        0x2D => Some(b'x'),
        0x2E => Some(b'c'),
        0x2F => Some(b'v'),
        0x30 => Some(b'b'),
        0x31 => Some(b'n'),
        0x32 => Some(b'm'),
        _ => None,
    }
}
