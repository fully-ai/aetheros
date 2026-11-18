#![cfg_attr(not(feature = "std"), no_std)]

//! AetherFS v1 On-Disk Specification and Crash-Consistent Engine (Milestone 7).
//!
//! Key architectural properties:
//! - Designed minimum capacity >= **1 PiB** (`2^50` bytes = `1,125,899,906,842,624` bytes,
//!   or `274,877,906,944` 4 KiB blocks) using 64-bit block indices and checked arithmetic.
//! - Primary Superblock at Block 1 (`4096`) and mirrored Backup Superblock at Block 2 (`8192`),
//!   protected by Castagnoli CRC32C checksums with automatic failover recovery.
//! - 256-byte Protected Object Records (Inodes) embedding:
//!   - `object_uuid`, `owner_user_uuid`, `owner_group_uuid`, `execute_as_uuid`
//!   - `uid`, `gid`, `mode`, `flags`, `nlink`, `size_bytes` (64-bit)
//!   - 4 inline 64-bit sparse-capable extents (`logical_block`, `phys_block`, `block_count`)
//!   - External metadata/path/ACL block pointer (`meta_block`)
//!   - Inode CRC32C checksum
//! - Write-Ahead Journal (`AETHJRNL`) supporting atomic multi-block transactions,
//!   committed transaction replay on crash recovery, and torn/corrupted transaction rollback.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use aether_uuid::Uuid;

pub const BLOCK_SIZE: usize = 4096;
pub const SUPERBLOCK_MAGIC: [u8; 8] = *b"AETHERFS";
pub const JOURNAL_MAGIC: [u8; 8] = *b"AETHJRNL";
pub const FORMAT_VERSION_MAJOR: u16 = 1;
pub const FORMAT_VERSION_MINOR: u16 = 0;

/// 1 Pebibyte (`2^50` bytes = `1,125,899,906,842,624` bytes).
pub const ONE_PIB_BYTES: u64 = 1u64 << 50;
/// Number of 4 KiB blocks in 1 PiB (`2^38` = `274,877,906,944` blocks).
pub const ONE_PIB_BLOCKS_4K: u64 = ONE_PIB_BYTES / (BLOCK_SIZE as u64);
/// Maximum theoretical volume capacity with 64-bit 4 KiB block addressing (`u64::MAX` bytes).
pub const MAX_ADDRESSABLE_BYTES: u64 = u64::MAX;

pub const PRIMARY_SUPERBLOCK_LBA: u64 = 1;
pub const BACKUP_SUPERBLOCK_LBA: u64 = 2;

pub const JOURNAL_COMMIT_MAGIC: u32 = 0xC04D_1700;

pub const FEAT_JOURNAL: u64 = 1 << 0;
pub const FEAT_CHECKSUMS: u64 = 1 << 1;
pub const FEAT_EXTENTS: u64 = 1 << 2;
pub const FEAT_UUID_ACL: u64 = 1 << 3;
pub const FEAT_EXECUTE_AS: u64 = 1 << 4;
pub const FEAT_SPARSE_1PIB: u64 = 1 << 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    IoError,
    InvalidMagic,
    UnsupportedVersion,
    ChecksumMismatch,
    CorruptedSuperblock,
    CorruptedJournal,
    ArithmeticOverflow,
    DiskFull,
    InodeTableFull,
    NotFound,
    AlreadyExists,
    PermissionDenied,
    InvalidArgument,
}

/// Abstract 4 KiB Block Device trait implemented by VirtIO-blk, RAM disks, and sparse host test disks.
pub trait BlockDevice {
    fn total_blocks(&self) -> u64;
    fn read_block(&self, lba: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError>;
    fn write_block(&mut self, lba: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), FsError>;
}

/// Pure software Castagnoli CRC32C (`0x82F63B78`) checksum computation.
pub fn crc32c(initial: u32, data: &[u8]) -> u32 {
    let mut crc = !initial;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0x82F6_3B78 & mask);
        }
    }
    !crc
}

/// Convert a 64-bit byte offset into `(logical_4k_block, offset_within_block)` using checked arithmetic.
/// Supports offsets `>= 1 PiB` (`2^50` bytes) without intermediate overflow.
#[inline]
pub fn byte_offset_to_block_checked(
    byte_offset: u64,
    len: u64,
) -> Result<(u64, usize, u64), FsError> {
    let end = byte_offset
        .checked_add(len)
        .ok_or(FsError::ArithmeticOverflow)?;
    let logical_block = byte_offset / (BLOCK_SIZE as u64);
    let in_block_off = (byte_offset % (BLOCK_SIZE as u64)) as usize;
    Ok((logical_block, in_block_off, end))
}

/// Verify that a volume of `total_blocks` can represent `byte_size` without overflow.
#[inline]
pub fn volume_capacity_bytes_checked(total_blocks: u64) -> Result<u64, FsError> {
    total_blocks
        .checked_mul(BLOCK_SIZE as u64)
        .ok_or(FsError::ArithmeticOverflow)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AetherSuperblock {
    pub magic: [u8; 8],
    pub version_major: u16,
    pub version_minor: u16,
    pub block_size: u32,
    pub volume_uuid: Uuid,
    pub total_blocks: u64,
    pub free_blocks: u64,
    pub inode_table_start: u64,
    pub inode_blocks: u64,
    pub max_inodes: u64,
    pub journal_start: u64,
    pub journal_blocks: u64,
    pub data_start: u64,
    pub next_free_data_block: u64,
    pub mount_generation: u64,
    pub last_journal_seq: u64,
    pub features: u64,
    pub checksum: u32,
}

impl AetherSuperblock {
    pub fn new(
        total_blocks: u64,
        inode_blocks: u64,
        journal_blocks: u64,
        volume_uuid: Uuid,
    ) -> Result<Self, FsError> {
        let min_blocks = 3u64
            .checked_add(inode_blocks)
            .and_then(|v| v.checked_add(journal_blocks))
            .and_then(|v| v.checked_add(4))
            .ok_or(FsError::ArithmeticOverflow)?;
        if total_blocks < min_blocks {
            return Err(FsError::InvalidArgument);
        }
        let inode_table_start = 3;
        let journal_start = inode_table_start + inode_blocks;
        let data_start = journal_start + journal_blocks;
        let free_blocks = total_blocks - data_start;

        let mut sb = Self {
            magic: SUPERBLOCK_MAGIC,
            version_major: FORMAT_VERSION_MAJOR,
            version_minor: FORMAT_VERSION_MINOR,
            block_size: BLOCK_SIZE as u32,
            volume_uuid,
            total_blocks,
            free_blocks,
            inode_table_start,
            inode_blocks,
            max_inodes: inode_blocks * 16, // 16 * 256B inodes per 4 KiB block
            journal_start,
            journal_blocks,
            data_start,
            next_free_data_block: data_start,
            mount_generation: 1,
            last_journal_seq: 0,
            features: FEAT_JOURNAL
                | FEAT_CHECKSUMS
                | FEAT_EXTENTS
                | FEAT_UUID_ACL
                | FEAT_EXECUTE_AS
                | FEAT_SPARSE_1PIB,
            checksum: 0,
        };
        sb.checksum = sb.compute_checksum();
        Ok(sb)
    }

    pub fn encode(&self, out: &mut [u8; BLOCK_SIZE]) {
        out.fill(0);
        out[0..8].copy_from_slice(&self.magic);
        out[8..10].copy_from_slice(&self.version_major.to_le_bytes());
        out[10..12].copy_from_slice(&self.version_minor.to_le_bytes());
        out[12..16].copy_from_slice(&self.block_size.to_le_bytes());
        out[16..32].copy_from_slice(self.volume_uuid.as_bytes());
        out[32..40].copy_from_slice(&self.total_blocks.to_le_bytes());
        out[40..48].copy_from_slice(&self.free_blocks.to_le_bytes());
        out[48..56].copy_from_slice(&self.inode_table_start.to_le_bytes());
        out[56..64].copy_from_slice(&self.inode_blocks.to_le_bytes());
        out[64..72].copy_from_slice(&self.max_inodes.to_le_bytes());
        out[72..80].copy_from_slice(&self.journal_start.to_le_bytes());
        out[80..88].copy_from_slice(&self.journal_blocks.to_le_bytes());
        out[88..96].copy_from_slice(&self.data_start.to_le_bytes());
        out[96..104].copy_from_slice(&self.next_free_data_block.to_le_bytes());
        out[104..112].copy_from_slice(&self.mount_generation.to_le_bytes());
        out[112..120].copy_from_slice(&self.last_journal_seq.to_le_bytes());
        out[120..128].copy_from_slice(&self.features.to_le_bytes());
        let csum = crc32c(0, &out[0..128]);
        out[128..132].copy_from_slice(&csum.to_le_bytes());
    }

    pub fn compute_checksum(&self) -> u32 {
        let mut buf = [0u8; BLOCK_SIZE];
        self.encode(&mut buf);
        u32::from_le_bytes(buf[128..132].try_into().unwrap())
    }

    pub fn decode(buf: &[u8; BLOCK_SIZE]) -> Result<Self, FsError> {
        if buf[0..8] != SUPERBLOCK_MAGIC {
            return Err(FsError::InvalidMagic);
        }
        let version_major = u16::from_le_bytes(buf[8..10].try_into().unwrap());
        let version_minor = u16::from_le_bytes(buf[10..12].try_into().unwrap());
        if version_major != FORMAT_VERSION_MAJOR {
            return Err(FsError::UnsupportedVersion);
        }
        let stored_csum = u32::from_le_bytes(buf[128..132].try_into().unwrap());
        let calc_csum = crc32c(0, &buf[0..128]);
        if stored_csum != calc_csum {
            return Err(FsError::ChecksumMismatch);
        }
        let mut uuid_bytes = [0u8; 16];
        uuid_bytes.copy_from_slice(&buf[16..32]);

        Ok(Self {
            magic: SUPERBLOCK_MAGIC,
            version_major,
            version_minor,
            block_size: u32::from_le_bytes(buf[12..16].try_into().unwrap()),
            volume_uuid: Uuid::from_bytes(uuid_bytes),
            total_blocks: u64::from_le_bytes(buf[32..40].try_into().unwrap()),
            free_blocks: u64::from_le_bytes(buf[40..48].try_into().unwrap()),
            inode_table_start: u64::from_le_bytes(buf[48..56].try_into().unwrap()),
            inode_blocks: u64::from_le_bytes(buf[56..64].try_into().unwrap()),
            max_inodes: u64::from_le_bytes(buf[64..72].try_into().unwrap()),
            journal_start: u64::from_le_bytes(buf[72..80].try_into().unwrap()),
            journal_blocks: u64::from_le_bytes(buf[80..88].try_into().unwrap()),
            data_start: u64::from_le_bytes(buf[88..96].try_into().unwrap()),
            next_free_data_block: u64::from_le_bytes(buf[96..104].try_into().unwrap()),
            mount_generation: u64::from_le_bytes(buf[104..112].try_into().unwrap()),
            last_journal_seq: u64::from_le_bytes(buf[112..120].try_into().unwrap()),
            features: u64::from_le_bytes(buf[120..128].try_into().unwrap()),
            checksum: stored_csum,
        })
    }
}

/// 64-bit Extent descriptor mapping logical file blocks `[logical_block, logical_block + block_count)`
/// to physical volume blocks `[phys_block, phys_block + block_count)`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AetherExtent {
    pub logical_block: u64,
    pub phys_block: u64,
    pub block_count: u64,
}

/// UUID-based Access Control Entry stored in the inode's auxiliary metadata block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OnDiskAce {
    pub subject_uuid: [u8; 16],
    pub is_group: u8,
    pub allow_rwx: u8,
    pub deny_rwx: u8,
    pub _pad: u8,
}

/// 256-byte Protected On-Disk Object Record (Inode), CRC32C checksummed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AetherOnDiskInode {
    pub allocated: bool,
    pub inode_id: u64,
    pub object_uuid: Uuid,
    pub owner_user_uuid: Uuid,
    pub owner_group_uuid: Uuid,
    pub execute_as_uuid: Uuid,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub flags: u32,
    pub nlink: u32,
    pub extent_count: u16,
    pub acl_count: u16,
    pub size_bytes: u64,
    pub ctime_ns: u64,
    pub mtime_ns: u64,
    pub atime_ns: u64,
    pub extents: [AetherExtent; 4],
    pub meta_block: u64, // Stores full normalized path + OnDiskAce array
    pub checksum: u32,
}

impl Default for AetherOnDiskInode {
    fn default() -> Self {
        Self {
            allocated: false,
            inode_id: 0,
            object_uuid: Uuid::NIL,
            owner_user_uuid: Uuid::NIL,
            owner_group_uuid: Uuid::NIL,
            execute_as_uuid: Uuid::NIL,
            uid: 0,
            gid: 0,
            mode: 0,
            flags: 0,
            nlink: 0,
            extent_count: 0,
            acl_count: 0,
            size_bytes: 0,
            ctime_ns: 0,
            mtime_ns: 0,
            atime_ns: 0,
            extents: [AetherExtent::default(); 4],
            meta_block: 0,
            checksum: 0,
        }
    }
}

impl AetherOnDiskInode {
    pub fn encode(&self, out: &mut [u8; 256]) {
        out.fill(0);
        if !self.allocated {
            return;
        }
        out[0] = 1;
        out[8..16].copy_from_slice(&self.inode_id.to_le_bytes());
        out[16..32].copy_from_slice(self.object_uuid.as_bytes());
        out[32..48].copy_from_slice(self.owner_user_uuid.as_bytes());
        out[48..64].copy_from_slice(self.owner_group_uuid.as_bytes());
        out[64..80].copy_from_slice(self.execute_as_uuid.as_bytes());
        out[80..84].copy_from_slice(&self.uid.to_le_bytes());
        out[84..88].copy_from_slice(&self.gid.to_le_bytes());
        out[88..92].copy_from_slice(&self.mode.to_le_bytes());
        out[92..96].copy_from_slice(&self.flags.to_le_bytes());
        out[96..100].copy_from_slice(&self.nlink.to_le_bytes());
        out[100..102].copy_from_slice(&self.extent_count.to_le_bytes());
        out[102..104].copy_from_slice(&self.acl_count.to_le_bytes());
        out[104..112].copy_from_slice(&self.size_bytes.to_le_bytes());
        out[112..120].copy_from_slice(&self.ctime_ns.to_le_bytes());
        out[120..128].copy_from_slice(&self.mtime_ns.to_le_bytes());
        out[128..136].copy_from_slice(&self.atime_ns.to_le_bytes());
        for (i, ext) in self.extents.iter().enumerate() {
            let base = 136 + i * 24;
            out[base..base + 8].copy_from_slice(&ext.logical_block.to_le_bytes());
            out[base + 8..base + 16].copy_from_slice(&ext.phys_block.to_le_bytes());
            out[base + 16..base + 24].copy_from_slice(&ext.block_count.to_le_bytes());
        }
        out[232..240].copy_from_slice(&self.meta_block.to_le_bytes());
        let csum = crc32c(0, &out[0..252]);
        out[252..256].copy_from_slice(&csum.to_le_bytes());
    }

    pub fn decode(buf: &[u8; 256]) -> Result<Self, FsError> {
        if buf[0] == 0 {
            return Ok(Self::default());
        }
        let stored_csum = u32::from_le_bytes(buf[252..256].try_into().unwrap());
        let calc_csum = crc32c(0, &buf[0..252]);
        if stored_csum != calc_csum {
            return Err(FsError::ChecksumMismatch);
        }
        let mut obj = [0u8; 16];
        let mut own_u = [0u8; 16];
        let mut own_g = [0u8; 16];
        let mut exec_as = [0u8; 16];
        obj.copy_from_slice(&buf[16..32]);
        own_u.copy_from_slice(&buf[32..48]);
        own_g.copy_from_slice(&buf[48..64]);
        exec_as.copy_from_slice(&buf[64..80]);

        let mut extents = [AetherExtent::default(); 4];
        for (i, slot) in extents.iter_mut().enumerate() {
            let base = 136 + i * 24;
            *slot = AetherExtent {
                logical_block: u64::from_le_bytes(buf[base..base + 8].try_into().unwrap()),
                phys_block: u64::from_le_bytes(buf[base + 8..base + 16].try_into().unwrap()),
                block_count: u64::from_le_bytes(buf[base + 16..base + 24].try_into().unwrap()),
            };
        }

        Ok(Self {
            allocated: true,
            inode_id: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
            object_uuid: Uuid::from_bytes(obj),
            owner_user_uuid: Uuid::from_bytes(own_u),
            owner_group_uuid: Uuid::from_bytes(own_g),
            execute_as_uuid: Uuid::from_bytes(exec_as),
            uid: u32::from_le_bytes(buf[80..84].try_into().unwrap()),
            gid: u32::from_le_bytes(buf[84..88].try_into().unwrap()),
            mode: u32::from_le_bytes(buf[88..92].try_into().unwrap()),
            flags: u32::from_le_bytes(buf[92..96].try_into().unwrap()),
            nlink: u32::from_le_bytes(buf[96..100].try_into().unwrap()),
            extent_count: u16::from_le_bytes(buf[100..102].try_into().unwrap()),
            acl_count: u16::from_le_bytes(buf[102..104].try_into().unwrap()),
            size_bytes: u64::from_le_bytes(buf[104..112].try_into().unwrap()),
            ctime_ns: u64::from_le_bytes(buf[112..120].try_into().unwrap()),
            mtime_ns: u64::from_le_bytes(buf[120..128].try_into().unwrap()),
            atime_ns: u64::from_le_bytes(buf[128..136].try_into().unwrap()),
            extents,
            meta_block: u64::from_le_bytes(buf[232..240].try_into().unwrap()),
            checksum: stored_csum,
        })
    }
}

/// Recovery report returned by `AetherFsVolume::mount_and_recover`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub used_backup_superblock: bool,
    pub replayed_journal_tx: bool,
    pub rolled_back_torn_tx: bool,
    pub replayed_blocks: u32,
}

/// Live mounted AetherFS v1 volume manager over a `BlockDevice`.
pub struct AetherFsVolume<D: BlockDevice> {
    pub dev: D,
    pub sb: AetherSuperblock,
    pub last_recovery: RecoveryReport,
}

impl<D: BlockDevice> AetherFsVolume<D> {
    /// Format a block device with a fresh AetherFS v1 layout, writing both Primary and Backup Superblocks.
    pub fn format(
        mut dev: D,
        inode_blocks: u64,
        journal_blocks: u64,
        volume_uuid: Uuid,
    ) -> Result<Self, FsError> {
        let total = dev.total_blocks();
        let sb = AetherSuperblock::new(total, inode_blocks, journal_blocks, volume_uuid)?;
        let mut sb_buf = [0u8; BLOCK_SIZE];
        sb.encode(&mut sb_buf);

        // Write Primary (LBA 1) and Backup (LBA 2) superblocks
        dev.write_block(PRIMARY_SUPERBLOCK_LBA, &sb_buf)?;
        dev.write_block(BACKUP_SUPERBLOCK_LBA, &sb_buf)?;

        // Zero inode table and journal header block
        let zero_block = [0u8; BLOCK_SIZE];
        for lba in sb.inode_table_start..(sb.inode_table_start + sb.inode_blocks) {
            dev.write_block(lba, &zero_block)?;
        }
        dev.write_block(sb.journal_start, &zero_block)?;

        Ok(Self {
            dev,
            sb,
            last_recovery: RecoveryReport::default(),
        })
    }

    /// Mount an existing AetherFS v1 volume, failing over to Backup Superblock if Primary is
    /// damaged, and running Write-Ahead Journal crash recovery.
    pub fn mount_and_recover(mut dev: D) -> Result<Self, FsError> {
        let mut buf = [0u8; BLOCK_SIZE];
        let mut report = RecoveryReport::default();

        dev.read_block(PRIMARY_SUPERBLOCK_LBA, &mut buf)?;
        let mut sb = match AetherSuperblock::decode(&buf) {
            Ok(primary) => primary,
            Err(_) => {
                // Primary superblock corrupted! Recover from Backup Superblock at LBA 2
                dev.read_block(BACKUP_SUPERBLOCK_LBA, &mut buf)?;
                let backup =
                    AetherSuperblock::decode(&buf).map_err(|_| FsError::CorruptedSuperblock)?;
                // Repair Primary Superblock from verified Backup
                dev.write_block(PRIMARY_SUPERBLOCK_LBA, &buf)?;
                report.used_backup_superblock = true;
                backup
            }
        };

        // Inspect Write-Ahead Journal header at `sb.journal_start`
        let mut jhdr = [0u8; BLOCK_SIZE];
        dev.read_block(sb.journal_start, &mut jhdr)?;
        if jhdr[0..8] == JOURNAL_MAGIC {
            let seq = u64::from_le_bytes(jhdr[8..16].try_into().unwrap());
            let count = u32::from_le_bytes(jhdr[16..20].try_into().unwrap());
            let commit_flag = u32::from_le_bytes(jhdr[20..24].try_into().unwrap());
            let expected_payload_crc = u32::from_le_bytes(jhdr[24..28].try_into().unwrap());
            let header_crc = u32::from_le_bytes(jhdr[252..256].try_into().unwrap());
            let calc_hdr_crc = crc32c(0, &jhdr[0..252]);

            if commit_flag == JOURNAL_COMMIT_MAGIC
                && header_crc == calc_hdr_crc
                && count > 0
                && (count as u64) < sb.journal_blocks
            {
                // Verify CRC32C across all logged data blocks before replaying
                let mut payload_crc = 0u32;
                let mut block_buf = [0u8; BLOCK_SIZE];
                for i in 0..(count as usize) {
                    dev.read_block(sb.journal_start + 1 + (i as u64), &mut block_buf)?;
                    payload_crc = crc32c(payload_crc, &block_buf);
                }

                if payload_crc == expected_payload_crc {
                    // Valid committed transaction: replay every logged block to its target LBA!
                    for i in 0..(count as usize) {
                        let off = 32 + i * 8;
                        let target_lba = u64::from_le_bytes(jhdr[off..off + 8].try_into().unwrap());
                        dev.read_block(sb.journal_start + 1 + (i as u64), &mut block_buf)?;
                        dev.write_block(target_lba, &block_buf)?;
                    }
                    sb.last_journal_seq = seq;
                    report.replayed_journal_tx = true;
                    report.replayed_blocks = count;
                } else {
                    report.rolled_back_torn_tx = true;
                }
            } else {
                // Uncommitted or corrupted journal transaction (torn write before commit): discard & roll back!
                report.rolled_back_torn_tx = true;
            }

            // Clear journal header after replay/rollback
            let zero = [0u8; BLOCK_SIZE];
            dev.write_block(sb.journal_start, &zero)?;
        }

        sb.mount_generation += 1;
        let mut sb_out = [0u8; BLOCK_SIZE];
        sb.encode(&mut sb_out);
        dev.write_block(PRIMARY_SUPERBLOCK_LBA, &sb_out)?;
        dev.write_block(BACKUP_SUPERBLOCK_LBA, &sb_out)?;

        Ok(Self {
            dev,
            sb,
            last_recovery: report,
        })
    }

    pub fn sync_superblocks(&mut self) -> Result<(), FsError> {
        let mut buf = [0u8; BLOCK_SIZE];
        self.sb.encode(&mut buf);
        self.dev.write_block(PRIMARY_SUPERBLOCK_LBA, &buf)?;
        self.dev.write_block(BACKUP_SUPERBLOCK_LBA, &buf)?;
        Ok(())
    }

    pub fn alloc_data_blocks(&mut self, count: u64) -> Result<u64, FsError> {
        if self.sb.free_blocks < count {
            return Err(FsError::DiskFull);
        }
        let start = self.sb.next_free_data_block;
        let end = start
            .checked_add(count)
            .ok_or(FsError::ArithmeticOverflow)?;
        if end > self.sb.total_blocks {
            return Err(FsError::DiskFull);
        }
        self.sb.next_free_data_block = end;
        self.sb.free_blocks -= count;
        Ok(start)
    }

    /// Execute an atomic Write-Ahead Journal transaction over up to 16 `(target_lba, block_data)` pairs.
    /// If `simulate_power_loss_after_commit == true`, leaves the committed transaction in the
    /// journal WITHOUT writing the home blocks (used to test crash replay!).
    /// If `simulate_torn_uncommitted == true`, writes an uncommitted journal entry and aborts
    /// (used to test torn-write rollback!).
    pub fn execute_journal_tx(
        &mut self,
        writes: &[(u64, [u8; BLOCK_SIZE])],
        simulate_power_loss_after_commit: bool,
        simulate_torn_uncommitted: bool,
    ) -> Result<(), FsError> {
        if writes.is_empty() || writes.len() > 16 || (writes.len() as u64) >= self.sb.journal_blocks
        {
            return Err(FsError::InvalidArgument);
        }

        let seq = self.sb.last_journal_seq + 1;
        let mut payload_crc = 0u32;

        // Step 1: Write logged data blocks into journal slots 1..=N
        for (i, (_, data)) in writes.iter().enumerate() {
            self.dev
                .write_block(self.sb.journal_start + 1 + (i as u64), data)?;
            payload_crc = crc32c(payload_crc, data);
        }

        // Step 2: Build journal descriptor block
        let mut jhdr = [0u8; BLOCK_SIZE];
        jhdr[0..8].copy_from_slice(&JOURNAL_MAGIC);
        jhdr[8..16].copy_from_slice(&seq.to_le_bytes());
        jhdr[16..20].copy_from_slice(&(writes.len() as u32).to_le_bytes());
        let commit_val = if simulate_torn_uncommitted {
            0u32
        } else {
            JOURNAL_COMMIT_MAGIC
        };
        jhdr[20..24].copy_from_slice(&commit_val.to_le_bytes());
        jhdr[24..28].copy_from_slice(&payload_crc.to_le_bytes());
        for (i, (target_lba, _)) in writes.iter().enumerate() {
            let off = 32 + i * 8;
            jhdr[off..off + 8].copy_from_slice(&target_lba.to_le_bytes());
        }
        let hdr_crc = crc32c(0, &jhdr[0..252]);
        jhdr[252..256].copy_from_slice(&hdr_crc.to_le_bytes());

        // Step 3: Write journal header (atomic commit point!)
        self.dev.write_block(self.sb.journal_start, &jhdr)?;

        if simulate_torn_uncommitted || simulate_power_loss_after_commit {
            return Ok(());
        }

        // Step 4: Checkpoint — write blocks to their final home LBAs
        for (target_lba, data) in writes {
            self.dev.write_block(*target_lba, data)?;
        }

        // Step 5: Clear journal header and advance checkpoint sequence
        let zero = [0u8; BLOCK_SIZE];
        self.dev.write_block(self.sb.journal_start, &zero)?;
        self.sb.last_journal_seq = seq;
        self.sync_superblocks()
    }

    pub fn read_inode(&self, slot_idx: u64) -> Result<AetherOnDiskInode, FsError> {
        if slot_idx >= self.sb.max_inodes {
            return Err(FsError::InvalidArgument);
        }
        let block_lba = self.sb.inode_table_start + (slot_idx / 16);
        let in_block_idx = ((slot_idx % 16) as usize) * 256;
        let mut block = [0u8; BLOCK_SIZE];
        self.dev.read_block(block_lba, &mut block)?;
        let slice: &[u8; 256] = block[in_block_idx..in_block_idx + 256].try_into().unwrap();
        AetherOnDiskInode::decode(slice)
    }

    pub fn write_inode_journaled(
        &mut self,
        slot_idx: u64,
        inode: &AetherOnDiskInode,
    ) -> Result<(), FsError> {
        if slot_idx >= self.sb.max_inodes {
            return Err(FsError::InodeTableFull);
        }
        let block_lba = self.sb.inode_table_start + (slot_idx / 16);
        let in_block_idx = ((slot_idx % 16) as usize) * 256;
        let mut block = [0u8; BLOCK_SIZE];
        self.dev.read_block(block_lba, &mut block)?;

        let mut encoded = [0u8; 256];
        inode.encode(&mut encoded);
        block[in_block_idx..in_block_idx + 256].copy_from_slice(&encoded);

        self.execute_journal_tx(&[(block_lba, block)], false, false)
    }

    /// Encode path string + UUID ACL list into an auxiliary metadata block.
    pub fn encode_meta_block(path: &str, acls: &[OnDiskAce], out: &mut [u8; BLOCK_SIZE]) {
        out.fill(0);
        let pb = path.as_bytes();
        let plen = pb.len().min(512);
        out[0..2].copy_from_slice(&(plen as u16).to_le_bytes());
        out[2..4].copy_from_slice(&(acls.len().min(32) as u16).to_le_bytes());
        out[4..4 + plen].copy_from_slice(&pb[..plen]);
        for (i, ace) in acls.iter().take(32).enumerate() {
            let base = 516 + i * 20;
            out[base..base + 16].copy_from_slice(&ace.subject_uuid);
            out[base + 16] = ace.is_group;
            out[base + 17] = ace.allow_rwx;
            out[base + 18] = ace.deny_rwx;
            out[base + 19] = 0;
        }
        let csum = crc32c(0, &out[0..4092]);
        out[4092..4096].copy_from_slice(&csum.to_le_bytes());
    }

    pub fn decode_meta_block(buf: &[u8; BLOCK_SIZE]) -> Result<(String, Vec<OnDiskAce>), FsError> {
        let stored_csum = u32::from_le_bytes(buf[4092..4096].try_into().unwrap());
        if stored_csum != crc32c(0, &buf[0..4092]) {
            return Err(FsError::ChecksumMismatch);
        }
        let plen = (u16::from_le_bytes(buf[0..2].try_into().unwrap()) as usize).min(512);
        let acl_count = (u16::from_le_bytes(buf[2..4].try_into().unwrap()) as usize).min(32);
        let path_str =
            core::str::from_utf8(&buf[4..4 + plen]).map_err(|_| FsError::InvalidArgument)?;
        let mut acls = Vec::with_capacity(acl_count);
        for i in 0..acl_count {
            let base = 516 + i * 20;
            let mut subj = [0u8; 16];
            subj.copy_from_slice(&buf[base..base + 16]);
            acls.push(OnDiskAce {
                subject_uuid: subj,
                is_group: buf[base + 16],
                allow_rwx: buf[base + 17],
                deny_rwx: buf[base + 18],
                _pad: 0,
            });
        }
        Ok((String::from(path_str), acls))
    }

    /// Write data to an inode at arbitrary 64-bit `byte_offset` (supporting sparse files >= 1 PiB!).
    pub fn pwrite_inode(
        &mut self,
        slot_idx: u64,
        mut inode: AetherOnDiskInode,
        byte_offset: u64,
        data: &[u8],
    ) -> Result<AetherOnDiskInode, FsError> {
        if data.is_empty() {
            return Ok(inode);
        }
        let (first_logical, first_off, new_end) =
            byte_offset_to_block_checked(byte_offset, data.len() as u64)?;
        let last_logical = (new_end - 1) / (BLOCK_SIZE as u64);
        let num_blocks = (last_logical - first_logical) + 1;

        // Check if an existing extent already covers `[first_logical, first_logical + num_blocks)`
        let mut phys_start = None;
        for ext in inode.extents.iter().take(inode.extent_count as usize) {
            if first_logical >= ext.logical_block
                && first_logical + num_blocks <= ext.logical_block + ext.block_count
            {
                let delta = first_logical - ext.logical_block;
                phys_start = Some(ext.phys_block + delta);
                break;
            }
        }

        let phys_base = match phys_start {
            Some(p) => p,
            None => {
                if inode.extent_count as usize >= inode.extents.len() {
                    return Err(FsError::DiskFull);
                }
                let allocated_phys = self.alloc_data_blocks(num_blocks)?;
                let idx = inode.extent_count as usize;
                inode.extents[idx] = AetherExtent {
                    logical_block: first_logical,
                    phys_block: allocated_phys,
                    block_count: num_blocks,
                };
                inode.extent_count += 1;
                allocated_phys
            }
        };

        let mut remaining = data;
        let mut cur_in_block_off = first_off;
        for b in 0..num_blocks {
            let lba = phys_base + b;
            let mut blk = [0u8; BLOCK_SIZE];
            if cur_in_block_off != 0 || remaining.len() < BLOCK_SIZE {
                let _ = self.dev.read_block(lba, &mut blk);
            }
            let copy_len = remaining.len().min(BLOCK_SIZE - cur_in_block_off);
            blk[cur_in_block_off..cur_in_block_off + copy_len]
                .copy_from_slice(&remaining[..copy_len]);
            self.dev.write_block(lba, &blk)?;
            remaining = &remaining[copy_len..];
            cur_in_block_off = 0;
        }

        if new_end > inode.size_bytes {
            inode.size_bytes = new_end;
        }
        self.write_inode_journaled(slot_idx, &inode)?;
        Ok(inode)
    }

    /// Read data from an inode at arbitrary 64-bit `byte_offset` (unmapped sparse holes return 0x00).
    pub fn pread_inode(
        &self,
        inode: &AetherOnDiskInode,
        byte_offset: u64,
        dst: &mut [u8],
    ) -> Result<usize, FsError> {
        if byte_offset >= inode.size_bytes || dst.is_empty() {
            return Ok(0);
        }
        let avail = (inode.size_bytes - byte_offset).min(dst.len() as u64) as usize;
        let mut total_read = 0usize;

        while total_read < avail {
            let cur_off = byte_offset + (total_read as u64);
            let logical_blk = cur_off / (BLOCK_SIZE as u64);
            let in_blk_off = (cur_off % (BLOCK_SIZE as u64)) as usize;
            let chunk = (avail - total_read).min(BLOCK_SIZE - in_blk_off);

            let mut mapped_phys = None;
            for ext in inode.extents.iter().take(inode.extent_count as usize) {
                if logical_blk >= ext.logical_block
                    && logical_blk < ext.logical_block + ext.block_count
                {
                    mapped_phys = Some(ext.phys_block + (logical_blk - ext.logical_block));
                    break;
                }
            }

            if let Some(phys_lba) = mapped_phys {
                let mut blk = [0u8; BLOCK_SIZE];
                self.dev.read_block(phys_lba, &mut blk)?;
                dst[total_read..total_read + chunk]
                    .copy_from_slice(&blk[in_blk_off..in_blk_off + chunk]);
            } else {
                // Sparse hole: fill with zeroes!
                dst[total_read..total_read + chunk].fill(0);
            }

            total_read += chunk;
        }

        Ok(total_read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    /// Sparse in-memory 4 KiB block device capable of representing a virtual 2 PiB volume
    /// while only allocating touched 4 KiB blocks in a `BTreeMap<u64, [u8; 4096]>`.
    struct SparseTestDisk {
        total_blocks: u64,
        blocks: BTreeMap<u64, [u8; BLOCK_SIZE]>,
    }

    impl SparseTestDisk {
        fn new(total_blocks: u64) -> Self {
            Self {
                total_blocks,
                blocks: BTreeMap::new(),
            }
        }
    }

    impl BlockDevice for SparseTestDisk {
        fn total_blocks(&self) -> u64 {
            self.total_blocks
        }

        fn read_block(&self, lba: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError> {
            if lba >= self.total_blocks {
                return Err(FsError::IoError);
            }
            if let Some(b) = self.blocks.get(&lba) {
                buf.copy_from_slice(b);
            } else {
                buf.fill(0);
            }
            Ok(())
        }

        fn write_block(&mut self, lba: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), FsError> {
            if lba >= self.total_blocks {
                return Err(FsError::IoError);
            }
            self.blocks.insert(lba, *buf);
            Ok(())
        }
    }

    #[test]
    fn one_pib_capacity_and_sparse_file_beyond_1pib() {
        // 2 PiB virtual disk (2 * 274,877,906,944 = 549,755,813,888 blocks of 4 KiB)
        let two_pib_blocks = ONE_PIB_BLOCKS_4K * 2;
        assert_eq!(
            volume_capacity_bytes_checked(ONE_PIB_BLOCKS_4K),
            Ok(ONE_PIB_BYTES)
        );

        let disk = SparseTestDisk::new(two_pib_blocks);
        let vol_uuid = Uuid::deterministic(0xAE74, 1);
        let mut vol =
            AetherFsVolume::format(disk, 4, 18, vol_uuid).expect("format 2 PiB sparse volume");
        assert_eq!(vol.sb.total_blocks, two_pib_blocks);

        // Create an inode with UUID ownership and execute_as_uuid metadata
        let exec_target = Uuid::deterministic(1, 1000);
        let inode = AetherOnDiskInode {
            allocated: true,
            inode_id: 1,
            object_uuid: Uuid::deterministic(99, 1),
            owner_user_uuid: Uuid::ROOT_USER,
            owner_group_uuid: Uuid::ROOT_GROUP,
            execute_as_uuid: exec_target,
            uid: 0,
            gid: 0,
            mode: 0o100755,
            flags: 4,
            nlink: 1,
            ..AetherOnDiskInode::default()
        };

        // Write payload at byte offset 1 PiB + 8192 (1,125,899,906,850,816)!
        let offset_beyond_1pib = ONE_PIB_BYTES + 8192;
        let payload = b"AetherFS-1PiB-Sparse-Boundary-Verified!";
        let updated = vol
            .pwrite_inode(0, inode, offset_beyond_1pib, payload)
            .expect("pwrite at 1 PiB + 8 KiB");
        assert_eq!(
            updated.size_bytes,
            offset_beyond_1pib + payload.len() as u64
        );

        // Unmount and remount to verify on-disk persistence
        let raw_dev = vol.dev;
        let remounted = AetherFsVolume::mount_and_recover(raw_dev).expect("remount 2 PiB volume");
        let loaded = remounted.read_inode(0).expect("read_inode(0)");
        assert_eq!(loaded.execute_as_uuid, exec_target);
        assert_eq!(loaded.size_bytes, offset_beyond_1pib + payload.len() as u64);

        // Read from sparse hole at offset 4096 -> must be all zeroes
        let mut hole_buf = [0xFFu8; 32];
        let n_hole = remounted
            .pread_inode(&loaded, 4096, &mut hole_buf)
            .expect("pread sparse hole");
        assert_eq!(n_hole, 32);
        assert_eq!(hole_buf, [0u8; 32]);

        // Read back at 1 PiB + 8192 -> must match exact payload!
        let mut read_buf = [0u8; 39];
        let n_read = remounted
            .pread_inode(&loaded, offset_beyond_1pib, &mut read_buf)
            .expect("pread at 1 PiB + 8 KiB");
        assert_eq!(n_read, payload.len());
        assert_eq!(&read_buf, payload);
    }

    #[test]
    fn superblock_corruption_recovers_from_backup() {
        let disk = SparseTestDisk::new(1024);
        let vol_uuid = Uuid::deterministic(0xAE74, 2);
        let mut vol = AetherFsVolume::format(disk, 4, 18, vol_uuid).unwrap();

        // Corrupt Primary Superblock (LBA 1)
        let mut garbage = [0xDEu8; BLOCK_SIZE];
        garbage[0..8].copy_from_slice(&SUPERBLOCK_MAGIC);
        vol.dev
            .write_block(PRIMARY_SUPERBLOCK_LBA, &garbage)
            .unwrap();

        // Remount -> must detect CRC32C failure on LBA 1, recover from Backup LBA 2, and repair LBA 1!
        let recovered = AetherFsVolume::mount_and_recover(vol.dev).expect("recover from backup SB");
        assert!(recovered.last_recovery.used_backup_superblock);
        assert_eq!(recovered.sb.volume_uuid, vol_uuid);
    }

    #[test]
    fn journal_crash_recovery_replays_committed_and_rolls_back_torn() {
        let disk = SparseTestDisk::new(1024);
        let vol_uuid = Uuid::deterministic(0xAE74, 3);
        let mut vol = AetherFsVolume::format(disk, 4, 18, vol_uuid).unwrap();

        let target_lba = vol.sb.data_start;
        let mut committed_block = [0u8; BLOCK_SIZE];
        committed_block[0..12].copy_from_slice(b"COMMITTED_OK");

        // 1. Simulate power loss immediately AFTER journal commit (before home LBA write!)
        vol.execute_journal_tx(&[(target_lba, committed_block)], true, false)
            .unwrap();
        // Verify home LBA still has zeroes before recovery
        let mut pre_check = [0u8; BLOCK_SIZE];
        vol.dev.read_block(target_lba, &mut pre_check).unwrap();
        assert_eq!(&pre_check[0..12], &[0u8; 12]);

        // Mount & recover -> journal MUST replay committed transaction into `target_lba`!
        let mut vol = AetherFsVolume::mount_and_recover(vol.dev).unwrap();
        assert!(vol.last_recovery.replayed_journal_tx);
        assert_eq!(vol.last_recovery.replayed_blocks, 1);
        vol.dev.read_block(target_lba, &mut pre_check).unwrap();
        assert_eq!(&pre_check[0..12], b"COMMITTED_OK");

        // 2. Simulate a torn / uncommitted write in the journal
        let mut torn_block = [0u8; BLOCK_SIZE];
        torn_block[0..12].copy_from_slice(b"TORN_BAD_DAT");
        vol.execute_journal_tx(&[(target_lba, torn_block)], false, true)
            .unwrap();

        // Mount & recover -> journal MUST discard uncommitted transaction and preserve "COMMITTED_OK"!
        let vol = AetherFsVolume::mount_and_recover(vol.dev).unwrap();
        assert!(vol.last_recovery.rolled_back_torn_tx);
        assert!(!vol.last_recovery.replayed_journal_tx);
        vol.dev.read_block(target_lba, &mut pre_check).unwrap();
        assert_eq!(&pre_check[0..12], b"COMMITTED_OK");
    }

    #[test]
    fn corrupted_inode_checksum_detected() {
        let inode = AetherOnDiskInode {
            allocated: true,
            inode_id: 7,
            object_uuid: Uuid::ROOT_USER,
            owner_user_uuid: Uuid::ROOT_USER,
            owner_group_uuid: Uuid::ROOT_GROUP,
            execute_as_uuid: Uuid::NIL,
            uid: 0,
            gid: 0,
            mode: 0o100644,
            ..AetherOnDiskInode::default()
        };
        let mut raw = [0u8; 256];
        inode.encode(&mut raw);
        let decoded = AetherOnDiskInode::decode(&raw).expect("valid inode checksum");
        assert_eq!(decoded.inode_id, 7);
        assert_ne!(decoded.checksum, 0);

        // Flip 1 bit in uid field (attempting unauthorized tampering)
        raw[80] ^= 1;
        assert_eq!(
            AetherOnDiskInode::decode(&raw),
            Err(FsError::ChecksumMismatch)
        );
    }
}
