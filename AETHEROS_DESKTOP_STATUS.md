# AetherOS Graphical Desktop & Applications Status (`AETHEROS_DESKTOP_STATUS.md`)

- **Status**: **IMPLEMENTED & VERIFIED (`test_m14_framebuffer_compositor_and_gui` PASS at `1280x800` in QEMU UEFI & BIOS)**
- **Desktop Environment (`kernel/src/gui/mod.rs`)**:
  - `1280x800` 32-bit double-buffered software compositor with 60 FPS BGRX row-copy framebuffer blitting.
  - Live 8042 PS/2 Mouse (cursor movement, left-click window focus/buttons, title-bar window dragging, bottom-right corner window resizing, right-click desktop context menu) + full Set-1 QWERTY PS/2 Keyboard & UART console input.
  - Dark (`Aurora Slate`) & Light (`Daylight Breeze`) themes, 5 accent color palettes, Night Light filter, 4 virtual workspaces (`WS1..WS4`), bottom taskbar with `[AETHER]` Start Menu Launcher, Quick Settings panel, and Notification Center drawer.
- **8 Native Integrated Graphical Applications**:
  1. **Aether Files**: VFS & Removable USB (`/mnt/usb/AETHER_USB`) explorer with sidebar locations, `[Up ..]`, `[+Folder]`, `[+File]`, `[View:List/Grid]`, `[Eject/Mount USB]`, and UUID/ACL + content preview inspector.
  2. **Aether Browser**: Native web browser with bookmark tabs, `[TLS 1.3 OK]` / `[TLS BLOCKED]` badge, HTML DOM rendering, BadSSL certificate rejection, and `[Download]` to `/home/aether/Downloads/page.txt`.
  3. **Aether Terminal**: Interactive `/bin/aethersh` console with clickable quick-command chips (`help`, `lsblk`, `lsusb`, `ip addr`, `df`, `ps`, `aetherpkg`).
  4. **Aether Disks & USB**: Live block & partition table viewer with `[Mount USB]`, `[Safely Eject USB]`, and `[Run fsck All]`.
  5. **Aether Network & Firewall**: Live interface, ARP, DHCP, and TCP inspector with `[Renew DHCP]`, `[Firewall: ON/OFF]`, and `[eth0: UP/DOWN]`.
  6. **Aether Store**: Graphical `AetherPKG` client with one-click CRC32C-verified package installation.
  7. **Aether System Monitor**: Live CPU, Physical RAM (PMM), Kernel Heap, Network RX/TX, and AetherFS health meters.
  8. **Aether Settings & Device Manager**: Theme, accent, night light, `/proc/usb` hardware tree, and `[Save Config]` persistence to `/etc/desktop.conf`.
