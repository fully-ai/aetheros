//! Modular Network Stack (Ethernet II, ARP, IPv4, ICMP Echo, UDP, Loopback) — Milestone 12.
//!
//! All packet parsers enforce strict bounds checks and RFC 1071 16-bit one's complement
//! checksums so malformed or truncated network packets are safely dropped without panicking.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;

use crate::sync::IrqSpinLock;

pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;

pub const IPPROTO_ICMP: u8 = 1;
pub const IPPROTO_TCP: u8 = 6;
pub const IPPROTO_UDP: u8 = 17;

pub const ICMP_ECHO_REPLY: u8 = 0;
pub const ICMP_ECHO_REQUEST: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    PacketTooShort,
    UnsupportedEtherType,
    InvalidIpVersion,
    InvalidHeaderLength,
    TotalLengthExceedsBuffer,
    BadChecksum,
    UnsupportedProtocol,
    NoRoute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EthHeader {
    pub dst_mac: [u8; 6],
    pub src_mac: [u8; 6],
    pub ethertype: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Header {
    pub ihl_bytes: usize,
    pub total_len: u16,
    pub id: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src_ip: [u8; 4],
    pub dst_ip: [u8; 4],
}

/// Compute RFC 1071 16-bit one's complement Internet checksum.
pub fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0usize;
    while i + 1 < data.len() {
        let word = u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        sum = sum.wrapping_add(word);
        i += 2;
    }
    if i < data.len() {
        let word = u16::from_be_bytes([data[i], 0]) as u32;
        sum = sum.wrapping_add(word);
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Parse an Ethernet II frame into `(EthHeader, payload_slice)` with strict bounds validation.
pub fn parse_eth_frame(frame: &[u8]) -> Result<(EthHeader, &[u8]), NetError> {
    if frame.len() < 14 {
        return Err(NetError::PacketTooShort);
    }
    let mut dst_mac = [0u8; 6];
    let mut src_mac = [0u8; 6];
    dst_mac.copy_from_slice(&frame[0..6]);
    src_mac.copy_from_slice(&frame[6..12]);
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    Ok((
        EthHeader {
            dst_mac,
            src_mac,
            ethertype,
        },
        &frame[14..],
    ))
}

/// Parse and checksum-validate an IPv4 packet into `(Ipv4Header, transport_payload)`.
pub fn parse_ipv4_packet(pkt: &[u8]) -> Result<(Ipv4Header, &[u8]), NetError> {
    if pkt.len() < 20 {
        return Err(NetError::PacketTooShort);
    }
    let ver = pkt[0] >> 4;
    if ver != 4 {
        return Err(NetError::InvalidIpVersion);
    }
    let ihl = ((pkt[0] & 0x0F) as usize) * 4;
    if ihl < 20 || ihl > pkt.len() {
        return Err(NetError::InvalidHeaderLength);
    }
    let total_len = u16::from_be_bytes([pkt[2], pkt[3]]) as usize;
    if total_len < ihl || total_len > pkt.len() {
        return Err(NetError::TotalLengthExceedsBuffer);
    }

    // Verify IPv4 header checksum over `pkt[..ihl]` equals 0
    if internet_checksum(&pkt[..ihl]) != 0 {
        return Err(NetError::BadChecksum);
    }

    let mut src_ip = [0u8; 4];
    let mut dst_ip = [0u8; 4];
    src_ip.copy_from_slice(&pkt[12..16]);
    dst_ip.copy_from_slice(&pkt[16..20]);

    let hdr = Ipv4Header {
        ihl_bytes: ihl,
        total_len: total_len as u16,
        id: u16::from_be_bytes([pkt[4], pkt[5]]),
        ttl: pkt[8],
        protocol: pkt[9],
        checksum: u16::from_be_bytes([pkt[10], pkt[11]]),
        src_ip,
        dst_ip,
    };
    Ok((hdr, &pkt[ihl..total_len]))
}

/// Build a complete IPv4 + ICMP Echo Request or Reply packet.
pub fn build_ipv4_icmp_echo(
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    icmp_type: u8,
    ident: u16,
    seq: u16,
    payload: &[u8],
) -> Vec<u8> {
    let icmp_len = 8 + payload.len();
    let total_len = 20 + icmp_len;
    let mut pkt = alloc::vec![0u8; total_len];

    // IPv4 header
    pkt[0] = 0x45; // v4, IHL = 5 (20 bytes)
    pkt[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    pkt[4..6].copy_from_slice(&seq.to_be_bytes());
    pkt[8] = 64; // TTL
    pkt[9] = IPPROTO_ICMP;
    pkt[12..16].copy_from_slice(&src_ip);
    pkt[16..20].copy_from_slice(&dst_ip);
    let ip_csum = internet_checksum(&pkt[0..20]);
    pkt[10..12].copy_from_slice(&ip_csum.to_be_bytes());

    // ICMP header + payload
    let icmp = &mut pkt[20..];
    icmp[0] = icmp_type;
    icmp[1] = 0;
    icmp[4..6].copy_from_slice(&ident.to_be_bytes());
    icmp[6..8].copy_from_slice(&seq.to_be_bytes());
    icmp[8..8 + payload.len()].copy_from_slice(payload);
    let icmp_csum = internet_checksum(icmp);
    icmp[2..4].copy_from_slice(&icmp_csum.to_be_bytes());

    pkt
}

/// Build an IPv4 + UDP datagram packet.
pub fn build_ipv4_udp_packet(
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let udp_len = 8 + payload.len();
    let total_len = 20 + udp_len;
    let mut pkt = alloc::vec![0u8; total_len];

    pkt[0] = 0x45;
    pkt[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    pkt[8] = 64;
    pkt[9] = IPPROTO_UDP;
    pkt[12..16].copy_from_slice(&src_ip);
    pkt[16..20].copy_from_slice(&dst_ip);
    let ip_csum = internet_checksum(&pkt[0..20]);
    pkt[10..12].copy_from_slice(&ip_csum.to_be_bytes());

    let udp = &mut pkt[20..];
    udp[0..2].copy_from_slice(&src_port.to_be_bytes());
    udp[2..4].copy_from_slice(&dst_port.to_be_bytes());
    udp[4..6].copy_from_slice(&(udp_len as u16).to_be_bytes());
    udp[6..8].fill(0); // Optional IPv4 UDP checksum
    udp[8..8 + payload.len()].copy_from_slice(payload);

    pkt
}

pub type UdpDatagram = ([u8; 4], u16, Vec<u8>);

struct NetStack {
    local_mac: [u8; 6],
    local_ip: [u8; 4],
    loopback_ip: [u8; 4],
    arp_table: BTreeMap<[u8; 4], [u8; 6]>,
    udp_inbox: BTreeMap<u16, VecDeque<UdpDatagram>>,
    rx_packets: u64,
    tx_packets: u64,
    icmp_echo_replies: u64,
}

impl NetStack {
    const fn new() -> Self {
        Self {
            local_mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            local_ip: [10, 0, 2, 15],
            loopback_ip: [127, 0, 0, 1],
            arp_table: BTreeMap::new(),
            udp_inbox: BTreeMap::new(),
            rx_packets: 0,
            tx_packets: 0,
            icmp_echo_replies: 0,
        }
    }
}

static NET: IrqSpinLock<NetStack> = IrqSpinLock::new(NetStack::new());

pub fn init() {
    let mut net = NET.lock();
    net.arp_table
        .insert([10, 0, 2, 2], [0x52, 0x55, 0x0A, 0x00, 0x02, 0x02]);
    net.arp_table.insert([127, 0, 0, 1], [0; 6]);
}

/// Process an inbound IPv4 packet on loopback or virtual interface, returning an optional
/// outbound reply packet (e.g. ICMP Echo Reply).
pub fn process_inbound_ipv4(pkt: &[u8]) -> Result<Option<Vec<u8>>, NetError> {
    let (ip_hdr, payload) = parse_ipv4_packet(pkt)?;
    let mut net = NET.lock();
    net.rx_packets += 1;

    match ip_hdr.protocol {
        IPPROTO_ICMP => {
            if payload.len() < 8 {
                return Err(NetError::PacketTooShort);
            }
            if internet_checksum(payload) != 0 {
                return Err(NetError::BadChecksum);
            }
            let icmp_type = payload[0];
            let ident = u16::from_be_bytes([payload[4], payload[5]]);
            let seq = u16::from_be_bytes([payload[6], payload[7]]);
            if icmp_type == ICMP_ECHO_REQUEST {
                net.tx_packets += 1;
                net.icmp_echo_replies += 1;
                let reply = build_ipv4_icmp_echo(
                    ip_hdr.dst_ip,
                    ip_hdr.src_ip,
                    ICMP_ECHO_REPLY,
                    ident,
                    seq,
                    &payload[8..],
                );
                return Ok(Some(reply));
            }
            Ok(None)
        }
        IPPROTO_UDP => {
            if payload.len() < 8 {
                return Err(NetError::PacketTooShort);
            }
            let src_port = u16::from_be_bytes([payload[0], payload[1]]);
            let dst_port = u16::from_be_bytes([payload[2], payload[3]]);
            let udp_len = u16::from_be_bytes([payload[4], payload[5]]) as usize;
            if udp_len < 8 || udp_len > payload.len() {
                return Err(NetError::TotalLengthExceedsBuffer);
            }
            let data = payload[8..udp_len].to_vec();
            net.udp_inbox
                .entry(dst_port)
                .or_default()
                .push_back((ip_hdr.src_ip, src_port, data));
            Ok(None)
        }
        _ => Err(NetError::UnsupportedProtocol),
    }
}

pub fn recv_udp_datagram(port: u16) -> Option<([u8; 4], u16, Vec<u8>)> {
    NET.lock().udp_inbox.get_mut(&port)?.pop_front()
}

pub fn stats() -> (u64, u64, u64) {
    let net = NET.lock();
    (net.rx_packets, net.tx_packets, net.icmp_echo_replies)
}

pub fn interface_config() -> ([u8; 6], [u8; 4], [u8; 4]) {
    let net = NET.lock();
    (net.local_mac, net.local_ip, net.loopback_ip)
}
