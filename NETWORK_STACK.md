# AetherOS Networking Stack Architecture

## 1. Protocol Layers (`kernel/src/net/mod.rs`)

AetherOS includes a deterministic, bounds-checked kernel networking subsystem implementing Ethernet II framing, ARP cache resolution, IPv4 packet validation, RFC 1071 ones'-complement checksumming, ICMP Echo Request/Reply (`ping`), and UDP datagram delivery.

### Configured Network Interfaces
| Interface | IPv4 Address | Netmask | Gateway | Hardware MAC | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `lo` | `127.0.0.1` | `255.0.0.0` | `127.0.0.1` | `00:00:00:00:00:00` | `UP, LOOPBACK` |
| `eth0` | `10.0.2.15` | `255.255.255.0` | `10.0.2.2` | `52:54:00:12:34:56` | `UP, QEMU_SLIRP` |

---

## 2. Packet Processing & Validation Pipeline

1. **Ethernet II Framing (`ETHERTYPE_IPV4 = 0x0800`, `ETHERTYPE_ARP = 0x0806`)**:
   - Validates 14-byte header `[dst_mac(6), src_mac(6), ethertype(2)]` before dispatching to ARP or IPv4 handlers.
2. **IPv4 Header Validation (`parse_ipv4_packet`)**:
   - Rejects truncated packets (`< 20` bytes), non-IPv4 version fields (`version != 4`), invalid header lengths (`IHL < 5` or `IHL * 4 > total_len`), and total length fields exceeding the received frame buffer (`NetError::PayloadOutOfBounds`).
   - Verifies the RFC 1071 16-bit ones'-complement header checksum across `pkt[..ihl_bytes]` (`inet_checksum(&pkt[..ihl_bytes]) == 0`).
3. **ICMP Echo Request -> Echo Reply (`PROTO_ICMP = 1`)**:
   - Validates ICMP header (`>= 8` bytes) and full-message RFC 1071 checksum.
   - For `ICMP_ECHO_REQUEST (8)`, constructs an `ICMP_ECHO_REPLY (0)` preserving `identifier`, `sequence`, and payload bytes, recomputes both ICMP and IPv4 checksums, and increments `icmp_echo_replies`.
4. **UDP Datagram Delivery (`PROTO_UDP = 17`)**:
   - Validates 8-byte UDP header (`src_port`, `dst_port`, `udp_len`, `checksum`) against IPv4 payload bounds and delivers the datagram to the kernel's per-port UDP inbox (`recv_udp_datagram(port)`).
5. **Adversarial Packet Fuzzing**:
   - Verified in `test_drivers_pci_rtc_and_network_stack` against 2,000 mutated and randomly truncated IPv4/ICMP frames with zero panics or out-of-bounds accesses.
