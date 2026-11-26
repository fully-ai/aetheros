# AetherOS Networking Architecture

See also [NETWORK_STACK.md](NETWORK_STACK.md).

## 1. Protocol Stack Diagram

```mermaid
flowchart BT
    ETH["Ethernet II Frame Layer (14B Header: dst_mac, src_mac, ethertype 0x0800 / 0x0806)"]
    ARP["ARP Cache & Resolution Table (10.0.2.2 -> 52:55:0a:00:02:02)"]
    IPV4["IPv4 Layer (20..60B Header, Version=4, IHL>=5, RFC 1071 Checksum Verification)"]
    ICMP["ICMP Layer (Proto 1: Echo Request type 8 -> Echo Reply type 0 + Checksum)"]
    UDP["UDP Layer (Proto 17: 8B Header src_port, dst_port, len -> Per-Port Datagram Inbox)"]

    ETH --> ARP
    ETH --> IPV4
    IPV4 --> ICMP
    IPV4 --> UDP
```

## 2. Verification & Fuzz Testing (`test_drivers_pci_rtc_and_network_stack`)
- Verified outbound/inbound ICMP Echo Request (`10.0.2.2 -> 10.0.2.15`, payload `"AetherOS-Ping-Payload"`) producing a valid RFC 1071 checksummed ICMP Echo Reply (`10.0.2.15 -> 10.0.2.2`).
- Verified UDP loopback datagram transmission and receipt on port `8080` (`127.0.0.1:40000 -> 127.0.0.1:8080`).
- Verified 2,000 mutated and randomly truncated IPv4/ICMP frames through `net::process_inbound_ipv4` with zero panics.
