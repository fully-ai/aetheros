# Changelog (`CHANGELOG.md`)

## [v0.2.0-beta.1] - 2026-11-21

### Added
- **Aether Desktop Environment (`kernel/src/gui/mod.rs`)**:
  - Upgraded `1280x800` 60 FPS double-buffered software compositor with Dark (`Aurora Slate`) & Light (`Daylight Breeze`) themes, 5 configurable accent color palettes, Night Light filter, 4 virtual workspaces (`WS1..WS4`), left desktop shortcut icons, right-click desktop context menu, bottom taskbar with `[AETHER]` Start Menu Application Launcher, Quick Settings Control Center, and System Notification Center Drawer.
  - Full multi-window manager supporting mouse click-to-focus, title-bar dragging, bottom-right corner resizing, minimize (`[-]`), maximize/restore (`[+]`), close (`[X]`), and workspace assignment.
  - 8 Native Integrated Graphical Applications: **Aether Files** (VFS & USB Explorer with UUID/ACL inspector and file preview), **Aether Browser** (HTML DOM & X.509 TLS verified web browser), **Aether Terminal** (interactive `/bin/aethersh` with quick-command chips), **Aether Disks & USB** (partition & mount manager), **Aether Network & Firewall**, **Aether Store** (`AetherPKG` GUI), **Aether System Monitor**, and **Aether Settings & Device Manager** (with persistence to `/etc/desktop.conf`).
- **USB Host, Mass Storage BOT/SCSI, Partition & FAT32/ISO9660 Subsystem (`kernel/src/usb/mod.rs`, `kernel/src/storage/mod.rs`)**:
  - PCI USB Host Controller enumeration (`xHCI`, `EHCI`, `OHCI`, `UHCI`) and strict bounds-checked USB descriptor parser.
  - USB Mass Storage Bulk-Only Transport (`CBW`/`CSW`) and SCSI Command Set (`INQUIRY`, `TEST UNIT READY`, `REQUEST SENSE`, `READ CAPACITY(10)`, `READ(10)`, `WRITE(10)`, `SYNCHRONIZE CACHE(10)`).
  - MBR and GPT partition table parsers and formatters with boundary validation.
  - FAT32 BPB/FSInfo/cluster-chain read/write support, ISO9660 (`CD001`) detection, safe label sanitization (`sanitize_volume_label`), automatic mounting of `/dev/sdc1` at `/mnt/usb/AETHER_USB` with `MS_NOSUID | MS_NODEV | MS_NOEXEC`, safe eject with cache flush, and `fsck`.
- **Complete Internet & Network Stack (`kernel/src/net/mod.rs`)**:
  - RFC 2131 4-step DHCP Client (`DISCOVER -> OFFER -> REQUEST -> ACK`), RFC 1035 DNS wire resolver with TTL cache, RFC 9293 TCP 3-way handshake & state machine, Stateful Packet Firewall, and HTTP/1.1 + HTTPS/TLS X.509 Certificate Verification client.
- **Expanded Command-Line Ecosystem (`userspace/bins/src/bin/`)**:
  - Added 35+ new utilities (`lsusb`, `lspci`, `lsblk`, `blkid`, `findmnt`, `eject`, `sync`, `fsck`, `mkfs`, `ip`, `ifconfig`, `ping`, `route`, `arp`, `dig`, `nslookup`, `host`, `dhclient`, `curl`, `wget`, `netstat`, `ss`, `hostname`, `lscpu`, `lsmem`, `vmstat`, `systemctl`, `journalctl`, `head`, `tail`, `less`, `more`, `basename`, `dirname`, `readlink`, `realpath`, `du`, `sort`, `uniq`, `sha256sum`, `md5sum`, `hexdump`) and shell command chaining (`&&`, `;`).

## [v0.1.0-alpha.1] - 2026-11-18
- Initial Milestones 0 through 14 release of AetherOS (`x86_64` Higher-Half Kernel, Bitmap PMM, 4-Level VMM, Coalescing Heap, Preemptive Scheduler, Ring 3 `SYSCALL` ABI v1, IPC, VFS, AetherFS v1 `>= 1 PiB`, UUID Security/ACLs, ELF64 Loader, `/sbin/init`, `/bin/aethersh`, `/bin/sudo`, `/bin/aetherpkg`).
