# AetherOS Display & 2D Desktop Compositor Architecture (Milestone 14)

## 1. "Kernel First, GUI Last" Engineering Discipline

Per Project Rule 0.1, AetherOS's graphical desktop subsystem (`kernel/src/gui/mod.rs`) was implemented **only after** Milestones 1 through 13 (boot, CPU init, PMM/VMM/heap, preemptive scheduler, Ring 3 user mode, Syscall ABI v1, IPC, VFS, AetherFS v1 1 PiB engine, UUID security/ACLs, ELF64 loader, `/sbin/init`, `/bin/aethersh`, coreutils, `/bin/aetherpkg`, `/bin/sudo`, drivers, networking, and security hardening) were completely implemented and verified in QEMU.

---

## 2. Double-Buffered 2D Software Compositor (`kernel/src/gui/mod.rs`)

### Architecture
- **Surface Backbuffer**: `Compositor` allocates a 32-bit truecolor (`0x00RRGGBB`) backbuffer (`800x500` pixels in UEFI GOP mode) so all drawing primitives (`fill_rect`, `draw_border`, `draw_char`, `draw_text`) render tear-free off-screen before blitting to the hardware linear framebuffer via `drivers::fbcon::blit_rgb_buffer`.
- **Z-Ordered Window Manager**:
  - Maintains `windows: Vec<Window>` and `z_order: Vec<u32>` (bottom-to-top stacking order).
  - Supports `create_window`, `focus_window` (raising the selected window to the top of `z_order` and highlighting its active title bar in `#0284C7`), `move_window`, `write_window_line`, and `set_window_progress`.
- **Input Event Routing (`dispatch_event`)**:
  - `GuiEvent::MouseDown { x, y, button }`: Performs top-down z-order hit testing (`w.rect.contains(x, y)`) and focuses/raises the clicked window.
  - `GuiEvent::MouseMove { x, y }`: Updates software cursor coordinates (`cursor_x`, `cursor_y`).
  - `GuiEvent::KeyPress(ch)`: Routes ASCII keystrokes to the currently focused top-most window.

---

## 3. Default Desktop Workspace & `/proc/gui` Inspection

On boot, `gui::init_desktop()` composites the top status bar and three live system windows:
1. **Window 1 — `AetherTerminal - /bin/aethersh (PID 13)`**: Displays terminal output from `uname -a`, `id`, and `aetherpkg verify`.
2. **Window 2 — `AetherOS System Monitor (/proc)`**: Renders live progress meters for Physical RAM (PMM), Kernel Heap utilization, and AetherFS v1 Journal Health.
3. **Window 3 — `AetherFS 1PiB & UUID Security Audit`**: Displays `/mnt/aether` volume health, Superblock LBA 1/LBA 2 verification, and live `[AUDIT]` event count.

Live compositor metrics and window geometry are exposed via `/proc/gui`:
```text
resolution:       800x500
windows:          3
frames_rendered:  1
events_routed:    3
last_frame_crc:   0x2011ebfc
window[1]:        title="AetherTerminal - /bin/aethersh (PID 13)" rect=(16,40,360x190) focused=false
window[3]:        title="AetherFS 1PiB & UUID Security Audit" rect=(110,242,520x140) focused=false
window[2]:        title="AetherOS System Monitor (/proc)" rect=(392,48,360x175) focused=true
```
