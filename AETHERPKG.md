# AetherPKG & Aether Store Specification (`AETHERPKG.md`)

See also [PACKAGE_MANAGER.md](PACKAGE_MANAGER.md).

## 1. Signed Repository Catalog (`userspace/bins/src/bin/aetherpkg.rs`, `AppKind::Store`)

| Package | Version | Dependencies | Installed Artifact | Purpose |
| :--- | :--- | :--- | :--- | :--- |
| `aether-base` | `0.1.0` | *(none)* | `/etc/aether-base.manifest` | Base system release manifest |
| `aether-devtools` | `0.1.0` | `aether-base` | `/etc/aether-devtools.conf` | Developer diagnostic & sysctl profile |
| `aether-netutils` | `0.1.0` | `aether-base` | `/etc/hosts` | Network stack & hosts configuration |
| `aether-usbutils` | `0.2.0` | `aether-base` | `/etc/usb-policy.conf` | USB BOT/SCSI & FAT32 automount rules |
| `aether-browser` | `0.2.0` | `aether-base`, `aether-netutils` | `/etc/browser.conf` | Aether Browser & X.509 TLS trust bundle |
| `aether-firewall` | `0.2.0` | `aether-base`, `aether-netutils` | `/etc/firewall.conf` | Stateful packet filter configuration |

## 2. CLI & GUI Operations
- CLI: `aetherpkg list`, `aetherpkg search <query>`, `aetherpkg info <pkg>`, `aetherpkg install <pkg>`, `aetherpkg remove <pkg>`, `aetherpkg verify`, `aetherpkg update`.
- GUI: **Aether Store** (`AppKind::Store`) displays live installation state and provides one-click `[INSTALL]` buttons with CRC32C verification and desktop notifications.
