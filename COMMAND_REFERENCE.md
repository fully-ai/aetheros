# AetherOS Command Reference (`COMMAND_REFERENCE.md`)

## 1. Filesystem & Directory Commands
- `ls [-l] [path]`, `cd [dir]`, `pwd`, `mkdir <dir>`, `rmdir <dir>`, `touch <file>`, `cat <file>`, `head <file>`, `tail <file>`, `less <file>`, `more <file>`, `cp <src> <dst>`, `mv <src> <dst>`, `rm <file>`, `stat <path>`, `df`, `du [path]`, `mount`, `umount <target>`, `findmnt`, `basename <path>`, `dirname <path>`, `readlink <path>`, `realpath <path>`, `sync`, `fsck`, `mkfs`.

## 2. USB, Storage & Hardware Commands
- `lsusb`: Lists USB host controllers and attached USB devices (`/proc/usb`).
- `lspci`: Lists PCI bus devices, vendor/device IDs, classes, IRQs, and BAR0 (`/proc/pci`).
- `lsblk` / `blkid`: Lists block devices (`sda`, `sda1`, `sdb`, `sdc`, `sdc1`), sizes, filesystems, labels, UUIDs, and mount points (`/proc/block`).
- `eject [mountpoint]`: Flushes filesystem buffers (`sync`) and safely unmounts removable USB storage.
- `lscpu`, `lsmem`, `vmstat`, `dmesg`: Displays CPU features, physical/heap memory metrics, and kernel logs.

## 3. Networking & Internet Commands
- `ip` / `ifconfig` / `netstat` / `ss` / `route` / `arp`: Displays interface states (`lo`, `eth0`), IPv4/IPv6 addresses, ARP table, DHCP lease, firewall status, and TCP connections (`/proc/net`).
- `ping [host]`: Sends ICMP Echo Request and verifies RFC 1071 checksummed Echo Reply.
- `dig <domain>` / `nslookup <domain>` / `host <domain>`: Queries the RFC 1035 DNS resolver (`10.0.2.3#53`).
- `dhclient`: Runs the 4-step RFC 2131 DHCP lease handshake (`DISCOVER -> OFFER -> REQUEST -> ACK`).
- `curl <url>` / `wget <url>`: Fetches HTTP/HTTPS resources with X.509 TLS certificate verification (`wget` saves to `/home/aether/Downloads/`).
- `hostname`: Prints `/etc/hostname`.

## 4. Process, User, Security & Package Commands
- `ps`, `kill <pid>`, `uname [-a]`, `uptime`, `free`, `systemctl`, `journalctl`.
- `id`, `whoami`, `groups`, `useradd <name> [uid]`, `sudo <cmd>`, `chmod <octal> <path>`, `chown <uid>[:<gid>] <path>`, `chgrp <gid> <path>`, `getfacl <path>`, `setfacl <uuid> <rwx> <path>`, `umask`.
- `aetherpkg <list|info|install|remove|verify|search|update|upgrade>`.
- `grep [-v] <pat> [file]`, `wc [-l|-w|-c] [file]`, `sort`, `uniq`, `sha256sum <file>`, `md5sum <file>`, `hexdump <file>`.
