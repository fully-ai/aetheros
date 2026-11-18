#![cfg_attr(not(feature = "std"), no_std)]

//! Stable 128-bit UUID identity primitives for AetherOS users, groups,
//! filesystem objects, and volumes.

use core::fmt;

/// A 128-bit RFC 4122 UUID.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Uuid([u8; 16]);

impl Uuid {
    /// Nil UUID (`00000000-0000-0000-0000-000000000000`).
    /// Used in `execute_as_uuid` to denote "no execution identity override".
    pub const NIL: Self = Self([0u8; 16]);

    /// Reserved Root User UUID (`00000000-0000-4000-8000-000000000001`).
    /// Always maps to UID `0`.
    pub const ROOT_USER: Self = Self([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]);

    /// Reserved Root Group UUID (`00000000-0000-4000-8000-000000000000`).
    /// Always maps to GID `0`.
    pub const ROOT_GROUP: Self = Self([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]);

    /// Reserved Wheel/Admin Group UUID (`00000000-0000-4000-8000-00000000000a`, GID 10).
    pub const WHEEL_GROUP: Self = Self([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x0a,
    ]);

    #[inline]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    #[inline]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    #[inline]
    pub const fn is_nil(&self) -> bool {
        let mut i = 0;
        while i < 16 {
            if self.0[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }

    #[inline]
    pub const fn is_root_user(&self) -> bool {
        let mut i = 0;
        while i < 16 {
            if self.0[i] != Self::ROOT_USER.0[i] {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Construct a valid RFC 4122 Version 4 UUID from 16 bytes of entropy.
    pub const fn from_random_bytes(mut bytes: [u8; 16]) -> Self {
        bytes[6] = (bytes[6] & 0x0F) | 0x40; // version 4
        bytes[8] = (bytes[8] & 0x3F) | 0x80; // variant 1 (RFC 4122)
        Self(bytes)
    }

    /// Derive a deterministic RFC 4122 Version 4-formatted UUID from a namespace tag and 64-bit seed
    /// using SplitMix64 mixing (useful for reproducible system objects and tests).
    pub const fn deterministic(namespace: u64, id: u64) -> Self {
        let a = splitmix64(namespace ^ 0x9e37_79b9_7f4a_7c15);
        let b = splitmix64(id ^ a.rotate_left(17));
        let lo = splitmix64(a ^ b);
        let hi = splitmix64(b.wrapping_add(0xbf58_476d_1ce4_e5b9));
        let mut bytes = [0u8; 16];
        let lb = lo.to_be_bytes();
        let hb = hi.to_be_bytes();
        let mut i = 0;
        while i < 8 {
            bytes[i] = lb[i];
            bytes[8 + i] = hb[i];
            i += 1;
        }
        Self::from_random_bytes(bytes)
    }

    /// Parse a canonical hyphenated UUID string (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`).
    pub fn parse_str(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 36 {
            return None;
        }
        if b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
            return None;
        }
        let positions = [0, 2, 4, 6, 9, 11, 14, 16, 19, 21, 24, 26, 28, 30, 32, 34];
        let mut out = [0u8; 16];
        for (idx, &pos) in positions.iter().enumerate() {
            let hi = hex_nibble(b[pos])?;
            let lo = hex_nibble(b[pos + 1])?;
            out[idx] = (hi << 4) | lo;
        }
        Some(Self(out))
    }

    /// Format as lowercase canonical hyphenated ASCII bytes.
    pub fn to_ascii_buf(&self) -> [u8; 36] {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = [0u8; 36];
        let mut o = 0;
        for (i, &byte) in self.0.iter().enumerate() {
            if i == 4 || i == 6 || i == 8 || i == 10 {
                out[o] = b'-';
                o += 1;
            }
            out[o] = HEX[(byte >> 4) as usize];
            out[o + 1] = HEX[(byte & 0x0F) as usize];
            o += 2;
        }
        out
    }
}

const fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

const fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let buf = self.to_ascii_buf();
        let s = core::str::from_utf8(&buf).unwrap_or("00000000-0000-0000-0000-000000000000");
        f.write_str(s)
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uuid({})", self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_nil_invariants() {
        assert!(Uuid::NIL.is_nil());
        assert!(!Uuid::NIL.is_root_user());
        assert!(!Uuid::ROOT_USER.is_nil());
        assert!(Uuid::ROOT_USER.is_root_user());
        assert_ne!(Uuid::ROOT_USER, Uuid::ROOT_GROUP);
    }

    #[test]
    fn parse_and_format_roundtrip() {
        let s = "00000000-0000-4000-8000-000000000001";
        let parsed = Uuid::parse_str(s).expect("valid root uuid");
        assert_eq!(parsed, Uuid::ROOT_USER);
        let buf = parsed.to_ascii_buf();
        assert_eq!(core::str::from_utf8(&buf).unwrap(), s);
    }

    #[test]
    fn deterministic_is_distinct_and_rfc4122() {
        let u1 = Uuid::deterministic(1, 1000);
        let u2 = Uuid::deterministic(1, 1001);
        assert_ne!(u1, u2);
        assert_eq!(u1.as_bytes()[6] >> 4, 4);
        assert_eq!(u1.as_bytes()[8] >> 6, 2);
    }

    #[test]
    fn invalid_uuid_strings_rejected() {
        assert!(Uuid::parse_str("").is_none());
        assert!(Uuid::parse_str("000000000000040008000000000000000001").is_none());
        assert!(Uuid::parse_str("zzzzzzzz-0000-4000-8000-000000000001").is_none());
    }
}
