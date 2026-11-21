# AetherOS Settings & Persistence (`SETTINGS.md`)

## 1. Graphical Control Center (`AppKind::Settings` in `kernel/src/gui/mod.rs`)

The **Aether Settings & Hardware Device Manager** application allows interactive customization of:
- **Appearance Theme**: Toggle between **Dark Mode (`Aurora Slate`)** and **Light Mode (`Daylight Breeze`)**.
- **Accent Color Palette**: Cycle across 5 accent palettes (`Sky Blue`, `Emerald Green`, `Royal Violet`, `Amber Gold`, `Crimson Rose`).
- **Night Light**: Toggle warm low-blue-light display filtering.
- **Network & Firewall**: Enable/disable `eth0`, renew DHCP lease, and toggle the stateful packet firewall (via `Quick Settings` or `Aether Network Manager`).
- **USB & Removable Media**: Safely eject or mount `/dev/sdc1` (`/mnt/usb/AETHER_USB`) with `nosuid,nodev,noexec` enforcement.
- **Configuration Persistence**: Clicking `[Save Config]` writes the validated configuration to `/etc/desktop.conf` in the VFS (and syncs to persistent storage).
