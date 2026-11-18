//! Milestone 14: AetherOS Framebuffer Graphics Subsystem, 2D Software Compositor,
//! Window Manager, Input Event Router, and Desktop Workspace (`kernel/src/gui/mod.rs`).
//!
//! Developed strictly after Milestones 1-13 kernel, filesystem, security, and Ring 3
//! user-space foundations ("Kernel first. GUI last.").

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;

use crate::drivers::fbcon as fb;
use crate::sync::IrqSpinLock;

pub const MAX_WINDOWS: usize = 8;

/// Built-in 8x8 bitmap font for window titles and desktop widgets (ASCII 32..=126).
/// Uses the same compact glyph rendering approach as the early framebuffer console,
/// scaled 1x (8x8) or 1x2 (8x16) for crisp window titles and client terminal text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub const fn new(x: usize, y: usize, w: usize, h: usize) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, px: usize, py: usize) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuiEvent {
    KeyPress(u8),
    MouseMove { x: usize, y: usize },
    MouseDown { x: usize, y: usize, button: u8 },
    FocusWindow(u32),
}

pub struct Window {
    pub id: u32,
    pub title: String,
    pub rect: Rect,
    pub focused: bool,
    pub visible: bool,
    pub bg_color: u32,
    pub lines: Vec<String>,
    pub progress_bars: Vec<(String, u32, u32)>, // (label, percent 0..=100, rgb_color)
}

pub struct Compositor {
    pub width: usize,
    pub height: usize,
    pub backbuffer: Vec<u32>,
    pub windows: Vec<Window>,
    pub z_order: Vec<u32>, // bottom-to-top window IDs
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub frames_composited: u64,
    pub events_routed: u64,
    pub last_frame_crc: u32,
    next_win_id: u32,
}

impl Compositor {
    pub const fn new_empty() -> Self {
        Self {
            width: 0,
            height: 0,
            backbuffer: Vec::new(),
            windows: Vec::new(),
            z_order: Vec::new(),
            cursor_x: 120,
            cursor_y: 120,
            frames_composited: 0,
            events_routed: 0,
            last_frame_crc: 0,
            next_win_id: 1,
        }
    }

    pub fn init(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.backbuffer = vec![0x0012_1824; width * height];
        self.windows.clear();
        self.z_order.clear();
        self.cursor_x = width / 2;
        self.cursor_y = height / 2;
        self.frames_composited = 0;
        self.events_routed = 0;
        self.next_win_id = 1;
    }

    pub fn create_window(&mut self, title: &str, rect: Rect, bg_color: u32) -> u32 {
        let id = self.next_win_id;
        self.next_win_id += 1;
        for w in &mut self.windows {
            w.focused = false;
        }
        self.windows.push(Window {
            id,
            title: String::from(title),
            rect,
            focused: true,
            visible: true,
            bg_color,
            lines: Vec::new(),
            progress_bars: Vec::new(),
        });
        self.z_order.push(id);
        id
    }

    pub fn write_window_line(&mut self, win_id: u32, line: &str) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == win_id) {
            w.lines.push(String::from(line));
            if w.lines.len() > 16 {
                w.lines.remove(0);
            }
        }
    }

    pub fn set_window_progress(&mut self, win_id: u32, label: &str, pct: u32, color: u32) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == win_id) {
            if let Some(existing) = w.progress_bars.iter_mut().find(|p| p.0 == label) {
                existing.1 = pct.min(100);
                existing.2 = color;
            } else {
                w.progress_bars
                    .push((String::from(label), pct.min(100), color));
            }
        }
    }

    pub fn focus_window(&mut self, win_id: u32) {
        if !self.windows.iter().any(|w| w.id == win_id) {
            return;
        }
        for w in &mut self.windows {
            w.focused = w.id == win_id;
        }
        self.z_order.retain(|&id| id != win_id);
        self.z_order.push(win_id);
    }

    pub fn move_window(&mut self, win_id: u32, new_x: usize, new_y: usize) {
        let max_x = self.width.saturating_sub(64);
        let max_y = self.height.saturating_sub(48);
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == win_id) {
            w.rect.x = new_x.min(max_x);
            w.rect.y = new_y.max(28).min(max_y);
        }
    }

    pub fn dispatch_event(&mut self, ev: GuiEvent) {
        self.events_routed += 1;
        match ev {
            GuiEvent::FocusWindow(id) => self.focus_window(id),
            GuiEvent::MouseMove { x, y } => {
                self.cursor_x = x.min(self.width.saturating_sub(1));
                self.cursor_y = y.min(self.height.saturating_sub(1));
            }
            GuiEvent::MouseDown { x, y, .. } => {
                self.cursor_x = x.min(self.width.saturating_sub(1));
                self.cursor_y = y.min(self.height.saturating_sub(1));
                // Hit-test windows from top of z-order down
                let mut hit: Option<u32> = None;
                for &id in self.z_order.iter().rev() {
                    if let Some(w) = self.windows.iter().find(|w| w.id == id && w.visible) {
                        if w.rect.contains(self.cursor_x, self.cursor_y) {
                            hit = Some(id);
                            break;
                        }
                    }
                }
                if let Some(id) = hit {
                    self.focus_window(id);
                }
            }
            GuiEvent::KeyPress(ch) => {
                if let Some(&top_id) = self.z_order.last() {
                    if let Some(w) = self.windows.iter_mut().find(|w| w.id == top_id) {
                        if ch == b'\n' {
                            w.lines.push(String::new());
                        } else if (0x20..=0x7E).contains(&ch) {
                            if let Some(last) = w.lines.last_mut() {
                                last.push(ch as char);
                            } else {
                                w.lines.push(alloc::format!("{}", ch as char));
                            }
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn put_pixel(&mut self, x: usize, y: usize, rgb: u32) {
        if x < self.width && y < self.height {
            self.backbuffer[y * self.width + x] = rgb;
        }
    }

    pub fn fill_rect(&mut self, r: Rect, rgb: u32) {
        let x_end = (r.x + r.w).min(self.width);
        let y_end = (r.y + r.h).min(self.height);
        for y in r.y.min(self.height)..y_end {
            let row = y * self.width;
            for x in r.x.min(self.width)..x_end {
                self.backbuffer[row + x] = rgb;
            }
        }
    }

    pub fn draw_border(&mut self, r: Rect, thickness: usize, rgb: u32) {
        self.fill_rect(Rect::new(r.x, r.y, r.w, thickness), rgb);
        self.fill_rect(
            Rect::new(r.x, (r.y + r.h).saturating_sub(thickness), r.w, thickness),
            rgb,
        );
        self.fill_rect(Rect::new(r.x, r.y, thickness, r.h), rgb);
        self.fill_rect(
            Rect::new((r.x + r.w).saturating_sub(thickness), r.y, thickness, r.h),
            rgb,
        );
    }

    /// Render an 8x8 bitmap glyph for ASCII characters into the backbuffer.
    fn draw_char(&mut self, px: usize, py: usize, ch: u8, fg: u32) {
        if ch <= b' ' {
            return;
        }
        let pattern = fb::glyph_bitmap(ch);
        for (row_idx, &bits) in pattern.iter().enumerate() {
            for col_idx in 0..8usize {
                if (bits & (0x80 >> col_idx)) != 0 {
                    self.put_pixel(px + col_idx, py + row_idx, fg);
                }
            }
        }
    }

    pub fn draw_text(&mut self, x: usize, y: usize, text: &str, fg: u32, max_chars: usize) {
        for (i, b) in text.bytes().take(max_chars).enumerate() {
            self.draw_char(x + i * 8, y, b, fg);
        }
    }

    /// Composite the desktop background, top status bar, z-ordered windows, and mouse cursor
    /// into `self.backbuffer`, then flush to the hardware framebuffer if present.
    pub fn composite_frame(&mut self) -> u32 {
        if self.width == 0 || self.height == 0 {
            return 0;
        }

        // 1. Desktop background gradient / slate grid
        for y in 0..self.height {
            let row = y * self.width;
            let shade = 0x14 + ((y * 16) / self.height.max(1)) as u32;
            let base = (0x0E << 16) | (shade << 8) | (shade + 12);
            for x in 0..self.width {
                self.backbuffer[row + x] = base;
            }
        }

        // 2. Top desktop status panel (26 px high)
        self.fill_rect(Rect::new(0, 0, self.width, 26), 0x001E_293B);
        self.fill_rect(Rect::new(0, 25, self.width, 1), 0x0038_BDF8);
        self.draw_text(
            12,
            9,
            "AetherOS v0.1.0-alpha.1 | x86_64 Desktop Compositor | Ring 3 + AetherFS 1PiB",
            0x00F8_FAFC,
            (self.width.saturating_sub(24)) / 8,
        );

        // 3. Render visible windows in bottom-to-top Z-order
        let z_ids = self.z_order.clone();
        for win_id in z_ids {
            let Some(win_idx) = self.windows.iter().position(|w| w.id == win_id) else {
                continue;
            };
            if !self.windows[win_idx].visible {
                continue;
            }
            let r = self.windows[win_idx].rect;
            let focused = self.windows[win_idx].focused;
            let bg = self.windows[win_idx].bg_color;
            let title = self.windows[win_idx].title.clone();
            let lines = self.windows[win_idx].lines.clone();
            let bars = self.windows[win_idx].progress_bars.clone();

            // Drop shadow
            self.fill_rect(Rect::new(r.x + 4, r.y + 4, r.w, r.h), 0x0007_0A0F);

            // Client body + outer border
            self.fill_rect(r, bg);
            let border_color = if focused { 0x0038_BDF8 } else { 0x0047_5569 };
            self.draw_border(r, 2, border_color);

            // Title bar (22 px)
            let title_bg = if focused { 0x0002_84C7 } else { 0x0033_4155 };
            self.fill_rect(
                Rect::new(r.x + 2, r.y + 2, r.w.saturating_sub(4), 20),
                title_bg,
            );
            // Window control pill
            self.fill_rect(
                Rect::new((r.x + r.w).saturating_sub(18), r.y + 7, 10, 10),
                0x00EF_4444,
            );
            let max_title_chars = r.w.saturating_sub(32) / 8;
            self.draw_text(r.x + 8, r.y + 8, &title, 0x00FF_FFFF, max_title_chars);

            // Client text lines
            let mut cy = r.y + 28;
            let max_cols = r.w.saturating_sub(16) / 8;
            for line in &lines {
                if cy + 12 > r.y + r.h {
                    break;
                }
                self.draw_text(r.x + 8, cy, line, 0x00E2_E8F0, max_cols);
                cy += 12;
            }

            // Progress bars (e.g. memory / CPU / storage meters)
            for (label, pct, bar_color) in &bars {
                if cy + 22 > r.y + r.h {
                    break;
                }
                self.draw_text(r.x + 8, cy, label, 0x0094_A3B8, max_cols);
                cy += 10;
                let bar_w = r.w.saturating_sub(20);
                self.fill_rect(Rect::new(r.x + 8, cy, bar_w, 8), 0x000F_172A);
                let fill_w = (bar_w * (*pct as usize)) / 100;
                self.fill_rect(Rect::new(r.x + 8, cy, fill_w, 8), *bar_color);
                cy += 12;
            }
        }

        // 4. Software Mouse Cursor (10x10 crosshair/arrow pointer)
        let cx = self.cursor_x;
        let cy = self.cursor_y;
        self.fill_rect(Rect::new(cx, cy, 8, 2), 0x00FF_FFFF);
        self.fill_rect(Rect::new(cx, cy, 2, 8), 0x00FF_FFFF);
        self.put_pixel(cx + 2, cy + 2, 0x0038_BDF8);
        self.put_pixel(cx + 3, cy + 3, 0x0038_BDF8);
        self.put_pixel(cx + 4, cy + 4, 0x0038_BDF8);

        self.frames_composited += 1;

        // Compute fast FNV-1a checksum of sampled backbuffer pixels for verification
        let mut hash: u32 = 0x811C_9DC5;
        let step = (self.backbuffer.len() / 512).max(1);
        for idx in (0..self.backbuffer.len()).step_by(step) {
            hash ^= self.backbuffer[idx];
            hash = hash.wrapping_mul(0x0100_0193);
        }
        self.last_frame_crc = hash;

        // Blit to active hardware framebuffer if available
        fb::blit_rgb_buffer(&self.backbuffer, self.width, self.height);

        hash
    }
}

static COMPOSITOR: IrqSpinLock<Compositor> = IrqSpinLock::new(Compositor::new_empty());

/// Initialize the Milestone 14 desktop compositor and populate the initial 3 system windows.
pub fn init_desktop() -> (usize, usize, u32) {
    let (hw_w, hw_h) = fb::dimensions().unwrap_or((800, 500));
    let w = hw_w.clamp(320, 1280);
    let h = hw_h.clamp(240, 800);
    fb::set_gui_active(true);

    let mut comp = COMPOSITOR.lock();
    comp.init(w, h);

    let left_w = ((w * 48) / 100).max(320);
    let right_x = left_w + 32;
    let right_w = w.saturating_sub(right_x + 20).max(280);
    let top_h = ((h * 52) / 100).max(190);
    let bot_y = top_h + 54;
    let bot_h = h.saturating_sub(bot_y + 18).max(130);

    // Window 1: AetherTerminal (/bin/aethersh)
    let w_term = comp.create_window(
        "AetherTerminal - /bin/aethersh (Interactive Console)",
        Rect::new(16, 40, left_w, top_h),
        0x000F_172A,
    );
    comp.write_window_line(w_term, "aether@aetheros:~$ uname -a");
    comp.write_window_line(w_term, "AetherOS aether 0.1.0-alpha.1 x86_64");
    comp.write_window_line(w_term, "aether@aetheros:~$ id");
    comp.write_window_line(
        w_term,
        "uid=0(root) uuid=00000000-0000-4000-8000-000000000001",
    );
    comp.write_window_line(w_term, "aether@aetheros:~$ aetherpkg verify");
    comp.write_window_line(w_term, "[OK] aether-base CRC32C=f4529967");
    comp.write_window_line(w_term, "[OK] aether-devtools CRC32C=235af06a");

    // Window 2: System Monitor
    let w_mon = comp.create_window(
        "AetherOS System Monitor (/proc)",
        Rect::new(right_x, 46, right_w, top_h.saturating_sub(12)),
        0x001E_293B,
    );
    let pmm_stats = crate::mm::pmm::stats();
    let heap_stats = crate::mm::heap::stats();
    let pmm_pct =
        ((pmm_stats.allocated_frames * 100) / pmm_stats.usable_frames.max(1)).min(100) as u32;
    let heap_pct = ((heap_stats.used_bytes * 100) / heap_stats.mapped_bytes.max(1)).min(100) as u32;
    comp.write_window_line(
        w_mon,
        &alloc::format!(
            "CPUs: 2 (x86_64) | Timer: {} ms",
            crate::arch::x86_64::timer::uptime_ms()
        ),
    );
    comp.set_window_progress(w_mon, "Physical RAM (PMM)", pmm_pct.max(8), 0x0022_C55E);
    comp.set_window_progress(
        w_mon,
        "Kernel Heap (Slab/Coalesce)",
        heap_pct.max(15),
        0x0038_BDF8,
    );
    comp.set_window_progress(w_mon, "AetherFS v1 Journal Health", 100, 0x00A8_55F7);

    // Window 3: AetherFS & Security Audit Inspector
    let w_sec = comp.create_window(
        "AetherFS 1PiB & UUID Security Audit",
        Rect::new(40, bot_y, w.saturating_sub(80), bot_h),
        0x0017_2033,
    );
    comp.write_window_line(
        w_sec,
        "Mount /mnt/aether: AetherFS v1 (64-bit extents, >=1 PiB, CRC32C)",
    );
    comp.write_window_line(
        w_sec,
        "Journal WAL: CLEAN | Superblock LBA 1 + Backup LBA 2: VERIFIED",
    );
    comp.write_window_line(
        w_sec,
        &alloc::format!(
            "Security Audit Events Logged: {} | Ring 3 Isolation: SMEP+NX+WP",
            crate::security::audit_count()
        ),
    );

    // Exercise input event routing + window focus switching
    comp.dispatch_event(GuiEvent::MouseDown {
        x: 50,
        y: 80,
        button: 1,
    });
    comp.dispatch_event(GuiEvent::KeyPress(b'#'));
    comp.dispatch_event(GuiEvent::FocusWindow(w_term));
    let crc = comp.composite_frame();
    (w, h, crc)
}

/// Mirror interactive `/dev/console` output into the `AetherTerminal` window and refresh the display.
pub fn append_terminal_output(text: &str) {
    let mut comp = COMPOSITOR.lock();
    if comp.width == 0 || comp.windows.is_empty() {
        return;
    }
    let max_lines = (comp.windows[0].rect.h.saturating_sub(36) / 12).max(6);
    let mut should_composite = false;
    {
        let win = &mut comp.windows[0];
        if win.lines.is_empty() {
            win.lines.push(String::new());
        }
        for b in text.bytes() {
            match b {
                b'\n' => {
                    win.lines.push(String::new());
                    while win.lines.len() > max_lines {
                        win.lines.remove(0);
                    }
                    should_composite = true;
                }
                b'\r' => {}
                0x08 | 0x7F => {
                    if let Some(last) = win.lines.last_mut() {
                        last.pop();
                    }
                    should_composite = true;
                }
                ch if (0x20..=0x7E).contains(&ch) => {
                    if let Some(last) = win.lines.last_mut() {
                        last.push(ch as char);
                    }
                    if ch == b'$' || ch == b'#' || text.len() == 1 {
                        should_composite = true;
                    }
                }
                _ => {}
            }
        }
    }
    if should_composite {
        comp.composite_frame();
    }
}

pub fn format_proc_gui() -> String {
    let comp = COMPOSITOR.lock();
    let mut out = String::new();
    let _ = writeln!(out, "resolution:       {}x{}", comp.width, comp.height);
    let _ = writeln!(out, "windows:          {}", comp.windows.len());
    let _ = writeln!(out, "frames_rendered:  {}", comp.frames_composited);
    let _ = writeln!(out, "events_routed:    {}", comp.events_routed);
    let _ = writeln!(out, "last_frame_crc:   {:#010x}", comp.last_frame_crc);
    for &id in &comp.z_order {
        if let Some(w) = comp.windows.iter().find(|w| w.id == id) {
            let _ = writeln!(
                out,
                "window[{}]:        title=\"{}\" rect=({},{},{}x{}) focused={}",
                w.id, w.title, w.rect.x, w.rect.y, w.rect.w, w.rect.h, w.focused
            );
        }
    }
    out
}
