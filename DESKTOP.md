# AetherOS Graphical Desktop Environment (`Aether Desktop v0.2.0`)

## 1. Visual Identity & Architecture (`kernel/src/gui/mod.rs`)

Aether Desktop is AetherOS's native 2D double-buffered graphical environment running at `1280x800` (clamped dynamically to the active UEFI GOP / VBE framebuffer resolution). Every widget, icon, menu, and window is rendered into an off-screen 32-bit RGB backbuffer and transferred to hardware video memory via a 60 FPS fast BGRX row-copy pipeline (`drivers::fbcon::blit_rgb_buffer`).

### Core Desktop Components
1. **Multi-Stop Wallpaper & Theme Engine**:
   - **Dark Theme (`Aurora Slate`)** and **Light Theme (`Daylight Breeze`)** with instant switching.
   - **5 Configurable Accent Palettes**: Sky Blue (`#0284C7`), Emerald Green (`#16A34A`), Royal Violet (`#7C3AED`), Amber Gold (`#D97706`), and Crimson Rose (`#E11D48`).
   - **Night Light Filter**: Optional warm amber low-blue-light filter for eye comfort.
   - **Persistent Configuration**: Saved to `/etc/desktop.conf` via `Aether Settings`.
2. **Top Workspace & System Status Bar (`26 px`)**:
   - **4 Virtual Workspaces (`[WS1] [WS2] [WS3] [WS4]`)**: Click any workspace button to switch active desktop workspaces.
   - **Live Hardware & Network Indicators**: Removable USB status (`USB:MOUNTED` / `USB:EJECTED`), Ethernet IPv4 status (`ETH0:10.0.2.15`), Audio volume (`VOL:80%`), and live kernel uptime (`UP:<sec>s`).
3. **Desktop Shortcut Icons (Left Vertical Rail)**:
   - 8 clickable shortcut icons (`Aether Files`, `Web Browser`, `Terminal`, `USB & Disks`, `Network Mgr`, `Aether Store`, `Sys Monitor`, `Settings`).
4. **Bottom Taskbar & System Tray (`34 px`)**:
   - **`[AETHER]` Start Button**: Toggles the Application Launcher popup menu.
   - **Running Window Taskbar Pills**: Displays active and minimized windows; clicking focuses, minimizes, or restores the target window.
   - **System Tray Controls**: `[USB:ON]`, `[NET]`, `[QUICK]` (Quick Settings Control Center), and `[BELL(N)]` (System Notification Center Drawer).
5. **Right-Click Desktop Context Menu**:
   - Right-clicking anywhere on the wallpaper opens a context menu (`Open Aether Files`, `Open Aether Browser`, `Open Aether Terminal`, `Toggle Dark / Light`, `Cycle Accent Color`).
