# Aether Network & Firewall Manager (`NETWORK_MANAGER.md`)

## 1. Overview (`kernel/src/net/mod.rs`, `AppKind::Network`)

The Aether Network Manager provides both a graphical control center (`Aether Network & Firewall Manager`) and a full CLI suite (`ip`, `ifconfig`, `dhclient`, `ping`, `dig`, `nslookup`, `host`, `route`, `arp`, `netstat`, `ss`, `curl`, `wget`) backed by `/proc/net`.

## 2. Capabilities
- **Interface Management**: Inspect and toggle `eth0` (`10.0.2.15/24`, IPv6 `fe80::5054:ff:fe12:3456/64`, MAC `52:54:00:12:34:56`) and `lo` (`127.0.0.1/8`, `::1/128`).
- **RFC 2131 DHCP Client**: Full 4-packet `DHCPDISCOVER -> DHCPOFFER -> DHCPREQUEST -> DHCPACK` lease acquisition and one-click `[Renew DHCP]` button with desktop notification.
- **RFC 1035 DNS Resolver**: Wire-format query/response packet builder and parser with TTL cache (`10.0.2.3#53`).
- **RFC 9293 TCP State Machine**: Tracks active TCP connections (`ESTABLISHED`, sequence/ack numbers, window size, RX/TX byte counters).
- **Stateful Packet Firewall**: One-click `[Firewall: ON/OFF]` switch blocking unauthorized inbound/outbound ports (`23`, `31337`).
