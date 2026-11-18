//! CMOS Real-Time Clock (RTC) and 8042 PS/2 Keyboard + Mouse Input Drivers.

use alloc::collections::VecDeque;

use crate::arch::x86_64::{inb, outb};
use crate::sync::IrqSpinLock;

const CMOS_ADDR: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

const PS2_DATA: u16 = 0x60;
const PS2_STATUS: u16 = 0x64;
const PS2_CMD: u16 = 0x64;

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

struct Ps2State {
    initialized: bool,
    shift_down: bool,
    mouse_cycle: u8,
    mouse_pkt: [u8; 3],
    key_queue: VecDeque<u8>,
}

impl Ps2State {
    const fn new() -> Self {
        Self {
            initialized: false,
            shift_down: false,
            mouse_cycle: 0,
            mouse_pkt: [0; 3],
            key_queue: VecDeque::new(),
        }
    }
}

static PS2: IrqSpinLock<Ps2State> = IrqSpinLock::new(Ps2State::new());

fn ps2_wait_write() {
    for _ in 0..10_000 {
        if (unsafe { inb(PS2_STATUS) } & 0x02) == 0 {
            return;
        }
        core::hint::spin_loop();
    }
}

fn ps2_wait_read() -> bool {
    for _ in 0..10_000 {
        if (unsafe { inb(PS2_STATUS) } & 0x01) != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn ps2_mouse_write(val: u8) {
    ps2_wait_write();
    unsafe { outb(PS2_CMD, 0xD4) };
    ps2_wait_write();
    unsafe { outb(PS2_DATA, val) };
    if ps2_wait_read() {
        let _ = unsafe { inb(PS2_DATA) }; // Consume ACK (0xFA)
    }
}

/// Enable the 8042 PS/2 keyboard and auxiliary mouse ports and start mouse packet streaming.
pub fn init_ps2_devices() {
    let mut ps2 = PS2.lock();
    if ps2.initialized {
        return;
    }

    // Drain any stale bytes in the 8042 output buffer
    for _ in 0..32 {
        if (unsafe { inb(PS2_STATUS) } & 0x01) == 0 {
            break;
        }
        let _ = unsafe { inb(PS2_DATA) };
    }

    // Enable first (keyboard) and second (mouse) PS/2 ports
    ps2_wait_write();
    unsafe { outb(PS2_CMD, 0xAE) };
    ps2_wait_write();
    unsafe { outb(PS2_CMD, 0xA8) };

    // Read controller configuration byte, ensure both clocks enabled (bits 4,5 clear)
    ps2_wait_write();
    unsafe { outb(PS2_CMD, 0x20) };
    if ps2_wait_read() {
        let mut cfg = unsafe { inb(PS2_DATA) };
        cfg &= !(0x10 | 0x20);
        ps2_wait_write();
        unsafe { outb(PS2_CMD, 0x60) };
        ps2_wait_write();
        unsafe { outb(PS2_DATA, cfg) };
    }

    // Reset mouse defaults and enable data reporting
    ps2_mouse_write(0xF6);
    ps2_mouse_write(0xF4);

    ps2.initialized = true;
}

/// Drain pending 8042 PS/2 keyboard and mouse bytes, queuing ASCII keys and dispatching mouse moves/clicks.
pub fn poll_ps2_events() {
    let mut mouse_events: [(u8, i16, i16); 16] = [(0, 0, 0); 16];
    let mut mouse_count = 0usize;

    {
        let mut ps2 = PS2.lock();
        if !ps2.initialized {
            drop(ps2);
            init_ps2_devices();
            ps2 = PS2.lock();
        }

        for _ in 0..64 {
            let status = unsafe { inb(PS2_STATUS) };
            if (status & 0x01) == 0 {
                break;
            }
            let data = unsafe { inb(PS2_DATA) };
            if (status & 0x20) != 0 {
                // PS/2 Mouse byte
                let idx = ps2.mouse_cycle as usize;
                if idx == 0 && (data & 0x08) == 0 {
                    // Wait for valid packet sync bit 3
                    continue;
                }
                ps2.mouse_pkt[idx] = data;
                ps2.mouse_cycle += 1;
                if ps2.mouse_cycle == 3 {
                    ps2.mouse_cycle = 0;
                    let flags = ps2.mouse_pkt[0];
                    // Ignore overflow packets
                    if (flags & 0xC0) == 0 {
                        let dx = (ps2.mouse_pkt[1] as i8) as i16;
                        let dy = (ps2.mouse_pkt[2] as i8) as i16;
                        let buttons = flags & 0x07;
                        if mouse_count < mouse_events.len() {
                            mouse_events[mouse_count] = (buttons, dx, dy);
                            mouse_count += 1;
                        }
                    }
                }
            } else {
                // PS/2 Keyboard Set-1 scancode
                match data {
                    0x2A | 0x36 => ps2.shift_down = true,
                    0xAA | 0xB6 => ps2.shift_down = false,
                    code if (code & 0x80) != 0 => {} // Other key release
                    code => {
                        if let Some(ch) = translate_scancode(code, ps2.shift_down) {
                            if ps2.key_queue.len() < 128 {
                                ps2.key_queue.push_back(ch);
                            }
                        }
                    }
                }
            }
        }
    }

    for &(buttons, dx, dy) in mouse_events.iter().take(mouse_count) {
        crate::gui::handle_ps2_mouse(buttons, dx, dy);
    }
}

/// Non-blocking poll of translated PS/2 ASCII keystrokes.
pub fn poll_ps2_ascii() -> Option<u8> {
    poll_ps2_events();
    PS2.lock().key_queue.pop_front()
}

fn translate_scancode(scancode: u8, shift: bool) -> Option<u8> {
    let ch = match scancode {
        0x01 => 0x1B, // ESC
        0x02 => {
            if shift {
                b'!'
            } else {
                b'1'
            }
        }
        0x03 => {
            if shift {
                b'@'
            } else {
                b'2'
            }
        }
        0x04 => {
            if shift {
                b'#'
            } else {
                b'3'
            }
        }
        0x05 => {
            if shift {
                b'$'
            } else {
                b'4'
            }
        }
        0x06 => {
            if shift {
                b'%'
            } else {
                b'5'
            }
        }
        0x07 => {
            if shift {
                b'^'
            } else {
                b'6'
            }
        }
        0x08 => {
            if shift {
                b'&'
            } else {
                b'7'
            }
        }
        0x09 => {
            if shift {
                b'*'
            } else {
                b'8'
            }
        }
        0x0A => {
            if shift {
                b'('
            } else {
                b'9'
            }
        }
        0x0B => {
            if shift {
                b')'
            } else {
                b'0'
            }
        }
        0x0C => {
            if shift {
                b'_'
            } else {
                b'-'
            }
        }
        0x0D => {
            if shift {
                b'+'
            } else {
                b'='
            }
        }
        0x0E => 0x08, // Backspace
        0x0F => b'\t',
        0x10 => {
            if shift {
                b'Q'
            } else {
                b'q'
            }
        }
        0x11 => {
            if shift {
                b'W'
            } else {
                b'w'
            }
        }
        0x12 => {
            if shift {
                b'E'
            } else {
                b'e'
            }
        }
        0x13 => {
            if shift {
                b'R'
            } else {
                b'r'
            }
        }
        0x14 => {
            if shift {
                b'T'
            } else {
                b't'
            }
        }
        0x15 => {
            if shift {
                b'Y'
            } else {
                b'y'
            }
        }
        0x16 => {
            if shift {
                b'U'
            } else {
                b'u'
            }
        }
        0x17 => {
            if shift {
                b'I'
            } else {
                b'i'
            }
        }
        0x18 => {
            if shift {
                b'O'
            } else {
                b'o'
            }
        }
        0x19 => {
            if shift {
                b'P'
            } else {
                b'p'
            }
        }
        0x1A => {
            if shift {
                b'{'
            } else {
                b'['
            }
        }
        0x1B => {
            if shift {
                b'}'
            } else {
                b']'
            }
        }
        0x1C => b'\n',
        0x1E => {
            if shift {
                b'A'
            } else {
                b'a'
            }
        }
        0x1F => {
            if shift {
                b'S'
            } else {
                b's'
            }
        }
        0x20 => {
            if shift {
                b'D'
            } else {
                b'd'
            }
        }
        0x21 => {
            if shift {
                b'F'
            } else {
                b'f'
            }
        }
        0x22 => {
            if shift {
                b'G'
            } else {
                b'g'
            }
        }
        0x23 => {
            if shift {
                b'H'
            } else {
                b'h'
            }
        }
        0x24 => {
            if shift {
                b'J'
            } else {
                b'j'
            }
        }
        0x25 => {
            if shift {
                b'K'
            } else {
                b'k'
            }
        }
        0x26 => {
            if shift {
                b'L'
            } else {
                b'l'
            }
        }
        0x27 => {
            if shift {
                b':'
            } else {
                b';'
            }
        }
        0x28 => {
            if shift {
                b'"'
            } else {
                b'\''
            }
        }
        0x2B => {
            if shift {
                b'|'
            } else {
                b'\\'
            }
        }
        0x2C => {
            if shift {
                b'Z'
            } else {
                b'z'
            }
        }
        0x2D => {
            if shift {
                b'X'
            } else {
                b'x'
            }
        }
        0x2E => {
            if shift {
                b'C'
            } else {
                b'c'
            }
        }
        0x2F => {
            if shift {
                b'V'
            } else {
                b'v'
            }
        }
        0x30 => {
            if shift {
                b'B'
            } else {
                b'b'
            }
        }
        0x31 => {
            if shift {
                b'N'
            } else {
                b'n'
            }
        }
        0x32 => {
            if shift {
                b'M'
            } else {
                b'm'
            }
        }
        0x33 => {
            if shift {
                b'<'
            } else {
                b','
            }
        }
        0x34 => {
            if shift {
                b'>'
            } else {
                b'.'
            }
        }
        0x35 => {
            if shift {
                b'?'
            } else {
                b'/'
            }
        }
        0x39 => b' ',
        _ => return None,
    };
    Some(ch)
}
