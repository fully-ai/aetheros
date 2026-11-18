#![cfg_attr(not(feature = "std"), no_std)]

//! Validated x86_64 ELF64 Executable Parser (Milestone 8).
//!
//! Security Invariants (Section 10 & 16.1):
//! - Zero `unsafe` code: all parsing uses checked slice bounds and `from_le_bytes`.
//! - Rejects any `PT_LOAD` segment that overflows 64-bit arithmetic, references bytes
//!   outside the ELF file buffer, has `p_filesz > p_memsz`, violates W^X (`PF_W | PF_X`),
//!   or targets addresses outside `[USER_ADDR_MIN, USER_ADDR_MAX)`.
//! - Verifies that `e_entry` resides inside an executable (`PF_X`) `PT_LOAD` segment.

use aether_abi::{USER_ADDR_MAX, USER_ADDR_MIN};

pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
pub const ELFCLASS64: u8 = 2;
pub const ELFDATA2LSB: u8 = 1;
pub const EV_CURRENT: u8 = 1;

pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;
pub const EM_X86_64: u16 = 62;

pub const PT_NULL: u32 = 0;
pub const PT_LOAD: u32 = 1;

pub const PF_X: u32 = 1 << 0;
pub const PF_W: u32 = 1 << 1;
pub const PF_R: u32 = 1 << 2;

pub const ELF64_EHDR_SIZE: usize = 64;
pub const ELF64_PHDR_SIZE: usize = 56;
pub const MAX_LOAD_SEGMENTS: usize = 16;
pub const MAX_PHDRS: usize = 32;
pub const MAX_SEGMENT_MEMSZ: u64 = 32 * 1024 * 1024; // 32 MiB per segment cap

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    BufferTooSmall,
    InvalidMagic,
    UnsupportedClass,
    UnsupportedEndianness,
    UnsupportedVersion,
    UnsupportedType,
    UnsupportedMachine,
    InvalidPhdrLayout,
    TooManyPhdrs,
    TooManyLoadSegments,
    NoLoadSegments,
    FileOffsetOutOfBounds,
    FileSizeExceedsMemSize,
    SegmentTooLarge,
    InvalidAlignment,
    KernelOrNullAddressTarget,
    WxSegmentViolation,
    EntryPointOutsideExecutableSegment,
    ArithmeticOverflow,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Elf64LoadSegment {
    pub vaddr: u64,
    pub memsz: u64,
    pub file_offset: usize,
    pub filesz: usize,
    pub flags: u32,
    pub align: u64,
}

impl Elf64LoadSegment {
    #[inline]
    pub const fn is_readable(&self) -> bool {
        (self.flags & PF_R) != 0
    }

    #[inline]
    pub const fn is_writable(&self) -> bool {
        (self.flags & PF_W) != 0
    }

    #[inline]
    pub const fn is_executable(&self) -> bool {
        (self.flags & PF_X) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatedElf64 {
    pub elf_type: u16,
    pub entry_point: u64,
    pub phdr_vaddr: u64,
    pub phnum: u16,
    pub load_count: usize,
    pub load_segments: [Elf64LoadSegment; MAX_LOAD_SEGMENTS],
    pub highest_vaddr_end: u64,
}

/// Parse and strictly validate an x86_64 ELF64 binary image from `bytes`.
pub fn parse_and_validate_elf64(bytes: &[u8]) -> Result<ValidatedElf64, ElfError> {
    if bytes.len() < ELF64_EHDR_SIZE {
        return Err(ElfError::BufferTooSmall);
    }
    if bytes[0..4] != ELF_MAGIC {
        return Err(ElfError::InvalidMagic);
    }
    if bytes[4] != ELFCLASS64 {
        return Err(ElfError::UnsupportedClass);
    }
    if bytes[5] != ELFDATA2LSB {
        return Err(ElfError::UnsupportedEndianness);
    }
    if bytes[6] != EV_CURRENT {
        return Err(ElfError::UnsupportedVersion);
    }

    let e_type = u16::from_le_bytes(bytes[16..18].try_into().unwrap());
    if e_type != ET_EXEC && e_type != ET_DYN {
        return Err(ElfError::UnsupportedType);
    }
    let e_machine = u16::from_le_bytes(bytes[18..20].try_into().unwrap());
    if e_machine != EM_X86_64 {
        return Err(ElfError::UnsupportedMachine);
    }
    let e_version = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
    if e_version != 1 {
        return Err(ElfError::UnsupportedVersion);
    }

    let e_entry = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
    let e_phoff = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
    let e_phentsize = u16::from_le_bytes(bytes[54..56].try_into().unwrap()) as usize;
    let e_phnum = u16::from_le_bytes(bytes[56..58].try_into().unwrap()) as usize;

    if e_phentsize != ELF64_PHDR_SIZE || e_phnum == 0 {
        return Err(ElfError::InvalidPhdrLayout);
    }
    if e_phnum > MAX_PHDRS {
        return Err(ElfError::TooManyPhdrs);
    }
    let ph_table_bytes = e_phnum
        .checked_mul(ELF64_PHDR_SIZE)
        .ok_or(ElfError::ArithmeticOverflow)?;
    let ph_table_end = e_phoff
        .checked_add(ph_table_bytes)
        .ok_or(ElfError::ArithmeticOverflow)?;
    if ph_table_end > bytes.len() {
        return Err(ElfError::InvalidPhdrLayout);
    }

    let mut load_segments = [Elf64LoadSegment::default(); MAX_LOAD_SEGMENTS];
    let mut load_count = 0usize;
    let mut highest_vaddr_end = 0u64;
    let mut entry_valid = false;
    let mut phdr_vaddr = 0u64;

    for i in 0..e_phnum {
        let off = e_phoff + i * ELF64_PHDR_SIZE;
        let ph = &bytes[off..off + ELF64_PHDR_SIZE];
        let p_type = u32::from_le_bytes(ph[0..4].try_into().unwrap());
        if p_type != PT_LOAD {
            continue;
        }

        let p_flags = u32::from_le_bytes(ph[4..8].try_into().unwrap());
        let p_offset = u64::from_le_bytes(ph[8..16].try_into().unwrap());
        let p_vaddr = u64::from_le_bytes(ph[16..24].try_into().unwrap());
        let p_filesz = u64::from_le_bytes(ph[32..40].try_into().unwrap());
        let p_memsz = u64::from_le_bytes(ph[40..48].try_into().unwrap());
        let p_align = u64::from_le_bytes(ph[48..56].try_into().unwrap());

        if p_memsz == 0 {
            continue;
        }
        if p_filesz > p_memsz {
            return Err(ElfError::FileSizeExceedsMemSize);
        }
        if p_memsz > MAX_SEGMENT_MEMSZ {
            return Err(ElfError::SegmentTooLarge);
        }

        // Enforce W^X: a segment must never be both writable and executable!
        if (p_flags & PF_W) != 0 && (p_flags & PF_X) != 0 {
            return Err(ElfError::WxSegmentViolation);
        }

        // Validate alignment power-of-two when > 1
        if p_align > 1
            && (!p_align.is_power_of_two() || (p_vaddr % p_align) != (p_offset % p_align))
        {
            return Err(ElfError::InvalidAlignment);
        }

        // Validate file offset and size fit inside `bytes`
        let file_end = p_offset
            .checked_add(p_filesz)
            .ok_or(ElfError::ArithmeticOverflow)?;
        if file_end > (bytes.len() as u64) {
            return Err(ElfError::FileOffsetOutOfBounds);
        }

        // Validate user virtual address range strictly within [USER_ADDR_MIN, USER_ADDR_MAX)
        let vaddr_end = p_vaddr
            .checked_add(p_memsz)
            .ok_or(ElfError::ArithmeticOverflow)?;
        if (p_vaddr as usize) < USER_ADDR_MIN || (vaddr_end as usize) > USER_ADDR_MAX {
            return Err(ElfError::KernelOrNullAddressTarget);
        }

        if (p_flags & PF_X) != 0 && e_entry >= p_vaddr && e_entry < vaddr_end {
            entry_valid = true;
        }
        if p_offset <= (e_phoff as u64) && file_end >= (ph_table_end as u64) {
            phdr_vaddr = p_vaddr + ((e_phoff as u64) - p_offset);
        }
        if vaddr_end > highest_vaddr_end {
            highest_vaddr_end = vaddr_end;
        }

        if load_count >= MAX_LOAD_SEGMENTS {
            return Err(ElfError::TooManyLoadSegments);
        }
        load_segments[load_count] = Elf64LoadSegment {
            vaddr: p_vaddr,
            memsz: p_memsz,
            file_offset: p_offset as usize,
            filesz: p_filesz as usize,
            flags: p_flags,
            align: p_align,
        };
        load_count += 1;
    }

    if load_count == 0 {
        return Err(ElfError::NoLoadSegments);
    }
    if !entry_valid {
        return Err(ElfError::EntryPointOutsideExecutableSegment);
    }

    Ok(ValidatedElf64 {
        elf_type: e_type,
        entry_point: e_entry,
        phdr_vaddr,
        phnum: e_phnum as u16,
        load_count,
        load_segments,
        highest_vaddr_end,
    })
}

/// Build a minimal valid static x86_64 ELF64 binary containing `code_bytes` (RX at `0x400000`)
/// and optional `rodata_bytes` (R at `0x401000`). Used by unit tests and kernel ELF loader verification.
pub fn build_minimal_elf64_image(code_bytes: &[u8], out_buf: &mut [u8]) -> usize {
    let total_len = 0x1000 + code_bytes.len();
    assert!(out_buf.len() >= total_len && code_bytes.len() <= 0x1000);
    out_buf[..total_len].fill(0);

    let entry_vaddr: u64 = 0x0000_0000_0040_1000;

    // ELF64 Header (64 bytes)
    out_buf[0..4].copy_from_slice(&ELF_MAGIC);
    out_buf[4] = ELFCLASS64;
    out_buf[5] = ELFDATA2LSB;
    out_buf[6] = EV_CURRENT;
    out_buf[16..18].copy_from_slice(&ET_EXEC.to_le_bytes());
    out_buf[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
    out_buf[20..24].copy_from_slice(&1u32.to_le_bytes());
    out_buf[24..32].copy_from_slice(&entry_vaddr.to_le_bytes());
    out_buf[32..40].copy_from_slice(&(ELF64_EHDR_SIZE as u64).to_le_bytes()); // e_phoff = 64
    out_buf[52..54].copy_from_slice(&(ELF64_EHDR_SIZE as u16).to_le_bytes());
    out_buf[54..56].copy_from_slice(&(ELF64_PHDR_SIZE as u16).to_le_bytes());
    out_buf[56..58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum = 1

    // Program Header #0 (PT_LOAD RX at offset 0x1000 -> vaddr 0x401000)
    let ph = &mut out_buf[64..64 + ELF64_PHDR_SIZE];
    ph[0..4].copy_from_slice(&PT_LOAD.to_le_bytes());
    ph[4..8].copy_from_slice(&(PF_R | PF_X).to_le_bytes());
    ph[8..16].copy_from_slice(&0x1000u64.to_le_bytes()); // p_offset
    ph[16..24].copy_from_slice(&entry_vaddr.to_le_bytes()); // p_vaddr
    ph[24..32].copy_from_slice(&entry_vaddr.to_le_bytes()); // p_paddr
    ph[32..40].copy_from_slice(&(code_bytes.len() as u64).to_le_bytes()); // p_filesz
    ph[40..48].copy_from_slice(&(code_bytes.len() as u64).to_le_bytes()); // p_memsz
    ph[48..56].copy_from_slice(&0x1000u64.to_le_bytes()); // p_align

    out_buf[0x1000..0x1000 + code_bytes.len()].copy_from_slice(code_bytes);
    total_len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_minimal_elf64_parses_cleanly() {
        let mut buf = [0u8; 4096 + 16];
        let code = [0x48, 0xC7, 0xC0, 0x46, 0x00, 0x00, 0x00, 0x0F, 0x05]; // sys_exit(0)
        let n = build_minimal_elf64_image(&code, &mut buf);
        let parsed = parse_and_validate_elf64(&buf[..n]).expect("valid ELF64");
        assert_eq!(parsed.entry_point, 0x0040_1000);
        assert_eq!(parsed.load_count, 1);
        assert!(parsed.load_segments[0].is_executable());
        assert!(!parsed.load_segments[0].is_writable());
    }

    #[test]
    fn rejects_malicious_kernel_address_segment() {
        let mut buf = [0u8; 4096 + 16];
        let code = [0x90; 8];
        let n = build_minimal_elf64_image(&code, &mut buf);
        // Tamper p_vaddr and e_entry to point to higher-half kernel address 0xffffffff80001000
        let kaddr: u64 = 0xFFFF_FFFF_8000_1000;
        buf[24..32].copy_from_slice(&kaddr.to_le_bytes());
        buf[64 + 16..64 + 24].copy_from_slice(&kaddr.to_le_bytes());
        assert_eq!(
            parse_and_validate_elf64(&buf[..n]),
            Err(ElfError::KernelOrNullAddressTarget)
        );
    }

    #[test]
    fn rejects_wx_segment_and_out_of_bounds_offset() {
        let mut buf = [0u8; 4096 + 16];
        let code = [0x90; 8];
        let n = build_minimal_elf64_image(&code, &mut buf);
        // Set PF_R | PF_W | PF_X simultaneously
        buf[64 + 4..64 + 8].copy_from_slice(&(PF_R | PF_W | PF_X).to_le_bytes());
        assert_eq!(
            parse_and_validate_elf64(&buf[..n]),
            Err(ElfError::WxSegmentViolation)
        );

        // Restore PF_R | PF_X, tamper p_filesz > file length
        buf[64 + 4..64 + 8].copy_from_slice(&(PF_R | PF_X).to_le_bytes());
        buf[64 + 32..64 + 40].copy_from_slice(&0x9000u64.to_le_bytes());
        buf[64 + 40..64 + 48].copy_from_slice(&0x9000u64.to_le_bytes());
        assert_eq!(
            parse_and_validate_elf64(&buf[..n]),
            Err(ElfError::FileOffsetOutOfBounds)
        );
    }

    #[test]
    fn fuzz_smoke_10000_mutated_elf_headers_never_panic() {
        let mut base = [0u8; 4096 + 16];
        let code = [0x90; 8];
        let n = build_minimal_elf64_image(&code, &mut base);

        let mut rng: u64 = 0x1234_5678_9ABC_DEF0;
        for iter in 0..10_000u64 {
            let mut mutated = base;
            // Mutate 1..4 random header bytes
            for _ in 0..((iter % 4) + 1) {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                let idx = (rng as usize) % 128;
                mutated[idx] ^= ((rng >> 16) as u8).max(1);
            }
            let slice_len = if iter % 10 == 0 {
                (rng as usize) % n
            } else {
                n
            };
            // Must return Ok or Err without ever panicking or overflowing
            let _ = parse_and_validate_elf64(&mutated[..slice_len]);
        }
    }
}
