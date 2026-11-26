# AetherOS Known Issues & Scope Boundaries (`AETHEROS_KNOWN_ISSUES.md`)

See also [KNOWN_ISSUES.md](KNOWN_ISSUES.md).

1. **Physical Wi-Fi (`802.11` WPA2/WPA3) Hardware Drivers**:
   - Per Section 5.6 ("Do not invent Wi-Fi support if the target driver is unavailable. Test the networking stack first using QEMU virtual Ethernet"), AetherOS implements its full DHCP, DNS, TCP/IP, HTTP/HTTPS, and Firewall stack over wired Ethernet (`eth0`, VirtIO-Net / Intel E1000) and clearly reports Wi-Fi / Bluetooth hardware as unsupported in virtualized QEMU x86_64 environments.
2. **SMP Application Processor (AP) Runqueues**:
   - The 1000 Hz preemptive scheduler runs on the Bootstrap Processor (BSP) with SMP CPU topology enumeration via Limine MP; per-AP runqueues are planned for `v0.3.0`.
3. **Static ELF64 Binaries**:
   - User-space binaries in `/bin` and `/sbin` are compiled as static freestanding `x86_64` ELF64 executables (`0x400000+`). Dynamic shared library loading (`PT_INTERP`) is planned for `v0.3.0`.
