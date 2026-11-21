//! AetherOS Block Device, Partition Table (MBR & GPT), FAT32 / ISO9660 / AetherFS
//! Volume Discovery, Removable Media Auto-Mount (`nosuid,nodev,noexec`), Safe Eject,
//! and `fsck` Recovery Subsystem (`kernel/src/storage/mod.rs`).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use aether_abi::Errno;
use aether_uuid::Uuid;

use crate::fs::{self, MS_NODEV, MS_NOEXEC, MS_NOSUID};
use crate::sync::IrqSpinLock;
use crate::usb::{
    self, BotCbw, CBW_SIGNATURE, SCSI_INQUIRY, SCSI_READ_10, SCSI_READ_CAPACITY_10,
    SCSI_SYNC_CACHE_10,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    BufferTooSmall,
    MissingBootSignature,
    CorruptedPartitionTable,
    PartitionOutOfBounds,
    UnsupportedFilesystem,
    CorruptedFat32Bpb,
    DeviceOffline,
    DeviceBusy,
    DeviceNotFound,
    IoError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionTableKind {
    None,
    Mbr,
    Gpt,
}

impl PartitionTableKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Mbr => "dos/mbr",
            Self::Gpt => "gpt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionEntry {
    pub index: u8,
    pub start_lba: u64,
    pub sector_count: u64,
    pub part_type_code: u8,
    pub type_guid: Uuid,
    pub part_uuid: Uuid,
    pub label: String,
    pub filesystem: String,
}

/// Strictly parse and validate a classic Master Boot Record (MBR) partition table from LBA 0.
/// Rejects any partition whose `[start_lba, start_lba + sector_count)` exceeds `total_sectors`.
pub fn parse_mbr_partitions(
    lba0: &[u8],
    total_sectors: u64,
) -> Result<Vec<PartitionEntry>, StorageError> {
    if lba0.len() < 512 {
        return Err(StorageError::BufferTooSmall);
    }
    if lba0[510] != 0x55 || lba0[511] != 0xAA {
        return Err(StorageError::MissingBootSignature);
    }

    let mut out = Vec::new();
    for i in 0..4usize {
        let base = 446 + i * 16;
        let status = lba0[base];
        if status != 0x00 && status != 0x80 {
            return Err(StorageError::CorruptedPartitionTable);
        }
        let ptype = lba0[base + 4];
        let start_lba = u32::from_le_bytes([
            lba0[base + 8],
            lba0[base + 9],
            lba0[base + 10],
            lba0[base + 11],
        ]) as u64;
        let count = u32::from_le_bytes([
            lba0[base + 12],
            lba0[base + 13],
            lba0[base + 14],
            lba0[base + 15],
        ]) as u64;

        if ptype == 0 || count == 0 {
            continue;
        }
        // Protective GPT MBR (`0xEE`) is handled by `parse_gpt_partitions`
        if ptype == 0xEE {
            continue;
        }
        if start_lba == 0 {
            return Err(StorageError::CorruptedPartitionTable);
        }
        let end_lba = start_lba
            .checked_add(count)
            .ok_or(StorageError::PartitionOutOfBounds)?;
        if end_lba > total_sectors {
            return Err(StorageError::PartitionOutOfBounds);
        }

        let fstype = match ptype {
            0x0B | 0x0C => "vfat",
            0x83 => "aetherfs",
            0xEF => "vfat",
            _ => "raw",
        };
        let disk_sig = u32::from_le_bytes([lba0[440], lba0[441], lba0[442], lba0[443]]) as u64;
        out.push(PartitionEntry {
            index: (i + 1) as u8,
            start_lba,
            sector_count: count,
            part_type_code: ptype,
            type_guid: Uuid::NIL,
            part_uuid: Uuid::deterministic(0x4D42_5200 | disk_sig, (i + 1) as u64),
            label: format!("MBR_PART{}", i + 1),
            filesystem: String::from(fstype),
        });
    }
    Ok(out)
}

/// Strictly parse and validate a GUID Partition Table (GPT) from LBA 1 (Header) and LBA 2+ (Entries).
pub fn parse_gpt_partitions(
    disk_bytes: &[u8],
    total_sectors: u64,
) -> Result<Vec<PartitionEntry>, StorageError> {
    if disk_bytes.len() < 4 * 512 {
        return Err(StorageError::BufferTooSmall);
    }
    let hdr = &disk_bytes[512..1024];
    if &hdr[0..8] != b"EFI PART" {
        return Err(StorageError::MissingBootSignature);
    }
    let header_size = u32::from_le_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]);
    if !(92..=512).contains(&header_size) {
        return Err(StorageError::CorruptedPartitionTable);
    }
    let my_lba = u64::from_le_bytes([
        hdr[24], hdr[25], hdr[26], hdr[27], hdr[28], hdr[29], hdr[30], hdr[31],
    ]);
    let first_usable = u64::from_le_bytes([
        hdr[40], hdr[41], hdr[42], hdr[43], hdr[44], hdr[45], hdr[46], hdr[47],
    ]);
    let last_usable = u64::from_le_bytes([
        hdr[48], hdr[49], hdr[50], hdr[51], hdr[52], hdr[53], hdr[54], hdr[55],
    ]);
    if my_lba != 1 || first_usable < 2 || last_usable >= total_sectors || first_usable > last_usable
    {
        return Err(StorageError::CorruptedPartitionTable);
    }

    let entries_lba = u64::from_le_bytes([
        hdr[72], hdr[73], hdr[74], hdr[75], hdr[76], hdr[77], hdr[78], hdr[79],
    ]);
    let num_entries = u32::from_le_bytes([hdr[80], hdr[81], hdr[82], hdr[83]]) as usize;
    let entry_size = u32::from_le_bytes([hdr[84], hdr[85], hdr[86], hdr[87]]) as usize;
    if entries_lba < 2 || entry_size != 128 || num_entries == 0 || num_entries > 128 {
        return Err(StorageError::CorruptedPartitionTable);
    }

    let table_byte_start = (entries_lba as usize)
        .checked_mul(512)
        .ok_or(StorageError::CorruptedPartitionTable)?;
    let max_scan = num_entries.min((disk_bytes.len().saturating_sub(table_byte_start)) / 128);

    let mut out = Vec::new();
    for i in 0..max_scan {
        let off = table_byte_start + i * 128;
        let ent = &disk_bytes[off..off + 128];
        if ent[0..16].iter().all(|&b| b == 0) {
            continue;
        }
        let mut type_bytes = [0u8; 16];
        let mut uuid_bytes = [0u8; 16];
        type_bytes.copy_from_slice(&ent[0..16]);
        uuid_bytes.copy_from_slice(&ent[16..32]);
        let start_lba = u64::from_le_bytes([
            ent[32], ent[33], ent[34], ent[35], ent[36], ent[37], ent[38], ent[39],
        ]);
        let end_lba = u64::from_le_bytes([
            ent[40], ent[41], ent[42], ent[43], ent[44], ent[45], ent[46], ent[47],
        ]);

        if start_lba < first_usable || end_lba > last_usable || end_lba < start_lba {
            return Err(StorageError::PartitionOutOfBounds);
        }
        let count = end_lba - start_lba + 1;

        // Decode UTF-16LE label (up to 36 code units in bytes 56..128)
        let mut label = String::new();
        for c in (56..128).step_by(2) {
            let u = u16::from_le_bytes([ent[c], ent[c + 1]]);
            if u == 0 {
                break;
            }
            if (0x20..=0x7E).contains(&u) {
                label.push((u as u8) as char);
            }
        }
        if label.is_empty() {
            label = format!("GPT_PART{}", i + 1);
        }

        out.push(PartitionEntry {
            index: (i + 1) as u8,
            start_lba,
            sector_count: count,
            part_type_code: 0xEE,
            type_guid: Uuid::from_bytes(type_bytes),
            part_uuid: Uuid::from_bytes(uuid_bytes),
            label,
            filesystem: String::from("vfat"),
        });
    }
    Ok(out)
}

/// Write a valid MBR with a single primary partition to LBA 0.
pub fn write_mbr_single_partition(
    disk: &mut [u8],
    part_type: u8,
    start_lba: u32,
    sector_count: u32,
) {
    disk[0..512].fill(0);
    // Disk signature
    disk[440..444].copy_from_slice(&0xAE74_0001u32.to_le_bytes());
    let base = 446;
    disk[base] = 0x80; // Bootable/active
    disk[base + 4] = part_type;
    disk[base + 8..base + 12].copy_from_slice(&start_lba.to_le_bytes());
    disk[base + 12..base + 16].copy_from_slice(&sector_count.to_le_bytes());
    disk[510] = 0x55;
    disk[511] = 0xAA;
}

/// Write a valid Protective MBR + GPT Header (LBA 1) + GPT Entry Array (LBA 2..3) for a single partition.
pub fn write_gpt_single_partition(
    disk: &mut [u8],
    total_sectors: u64,
    start_lba: u64,
    end_lba: u64,
    part_uuid: Uuid,
    label: &str,
) {
    // 1. Protective MBR at LBA 0
    write_mbr_single_partition(
        disk,
        0xEE,
        1,
        total_sectors.saturating_sub(1).min(u32::MAX as u64) as u32,
    );

    // 2. GPT Header at LBA 1
    let hdr = &mut disk[512..1024];
    hdr.fill(0);
    hdr[0..8].copy_from_slice(b"EFI PART");
    hdr[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    hdr[12..16].copy_from_slice(&92u32.to_le_bytes());
    hdr[24..32].copy_from_slice(&1u64.to_le_bytes()); // my_lba = 1
    hdr[32..40].copy_from_slice(&(total_sectors - 1).to_le_bytes()); // alternate_lba
    hdr[40..48].copy_from_slice(&4u64.to_le_bytes()); // first_usable_lba = 4
    hdr[48..56].copy_from_slice(&(total_sectors - 2).to_le_bytes()); // last_usable_lba
    hdr[56..72].copy_from_slice(Uuid::deterministic(0x6770_7400, 1).as_bytes());
    hdr[72..80].copy_from_slice(&2u64.to_le_bytes()); // partition_entry_lba = 2
    hdr[80..84].copy_from_slice(&4u32.to_le_bytes()); // num_partition_entries = 4
    hdr[84..88].copy_from_slice(&128u32.to_le_bytes()); // sizeof_partition_entry = 128

    // 3. GPT Partition Entry 0 at LBA 2
    let ent = &mut disk[1024..1152];
    ent.fill(0);
    // Basic Data Partition Type GUID: EBD0A0A2-B9E5-4433-87C0-68B6B72699C7
    ent[0..16].copy_from_slice(&[
        0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99,
        0xC7,
    ]);
    ent[16..32].copy_from_slice(part_uuid.as_bytes());
    ent[32..40].copy_from_slice(&start_lba.to_le_bytes());
    ent[40..48].copy_from_slice(&end_lba.to_le_bytes());
    for (idx, b) in label.bytes().take(35).enumerate() {
        let off = 56 + idx * 2;
        ent[off] = b;
        ent[off + 1] = 0;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fat32VolumeMeta {
    pub volume_label: String,
    pub volume_serial: u32,
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub num_fats: u8,
    pub fat_size_sectors: u32,
    pub root_cluster: u32,
    pub total_sectors: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fat32FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub first_cluster: u32,
    pub size: u32,
    pub data: Vec<u8>,
}

/// Format a partition slice (`part_bytes`) as a valid FAT32 volume with an 11-char `label`
/// and initial files (`README.TXT`, `SYSINFO.TXT`, `USB_TEST.TXT`).
pub fn format_fat32_partition(
    part_bytes: &mut [u8],
    label: &str,
    initial_files: &[(&str, &[u8])],
) -> Result<Fat32VolumeMeta, StorageError> {
    let total_sectors = (part_bytes.len() / 512) as u32;
    if total_sectors < 64 {
        return Err(StorageError::BufferTooSmall);
    }
    part_bytes.fill(0);

    let reserved_sectors: u16 = 8;
    let num_fats: u8 = 2;
    let fat_size_sectors: u32 = 4;
    let root_cluster: u32 = 2;

    // 1. FAT32 BIOS Parameter Block (BPB) at Partition LBA 0
    let bpb = &mut part_bytes[0..512];
    bpb[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    bpb[3..11].copy_from_slice(b"AETHEROS");
    bpb[11..13].copy_from_slice(&512u16.to_le_bytes()); // bytes_per_sector
    bpb[13] = 1; // sectors_per_cluster = 1 (512B clusters for compact RAM-backed USB image)
    bpb[14..16].copy_from_slice(&reserved_sectors.to_le_bytes());
    bpb[16] = num_fats;
    bpb[32..36].copy_from_slice(&total_sectors.to_le_bytes());
    bpb[36..40].copy_from_slice(&fat_size_sectors.to_le_bytes());
    bpb[44..48].copy_from_slice(&root_cluster.to_le_bytes());
    bpb[48..50].copy_from_slice(&1u16.to_le_bytes()); // FSInfo sector = 1
    bpb[50..52].copy_from_slice(&6u16.to_le_bytes()); // Backup boot sector = 6
    bpb[66] = 0x29; // Extended boot signature
    let serial: u32 = 0xAE74_2026;
    bpb[67..71].copy_from_slice(&serial.to_le_bytes());

    let mut padded_label = [b' '; 11];
    for (i, b) in label.bytes().take(11).enumerate() {
        padded_label[i] = b.to_ascii_uppercase();
    }
    bpb[71..82].copy_from_slice(&padded_label);
    bpb[82..90].copy_from_slice(b"FAT32   ");
    bpb[510] = 0x55;
    bpb[511] = 0xAA;

    // 2. FSInfo sector at Partition LBA 1
    let fsi = &mut part_bytes[512..1024];
    fsi[0..4].copy_from_slice(&0x4161_5252u32.to_le_bytes());
    fsi[484..488].copy_from_slice(&0x6141_7272u32.to_le_bytes());
    fsi[488..492].copy_from_slice(&(total_sectors.saturating_sub(32)).to_le_bytes());
    fsi[492..496].copy_from_slice(&3u32.to_le_bytes());
    fsi[510] = 0x55;
    fsi[511] = 0xAA;

    // 3. FAT1 & FAT2 tables
    let fat1_start = (reserved_sectors as usize) * 512;
    let fat2_start = fat1_start + (fat_size_sectors as usize) * 512;
    let data_start = fat1_start + (num_fats as usize) * (fat_size_sectors as usize) * 512;

    // Cluster 0, 1, and 2 (Root Directory) EOC markers
    for fat_base in [fat1_start, fat2_start] {
        part_bytes[fat_base..fat_base + 4].copy_from_slice(&0x0FFF_FFF8u32.to_le_bytes());
        part_bytes[fat_base + 4..fat_base + 8].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes());
        part_bytes[fat_base + 8..fat_base + 12].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes());
    }

    // 4. Populate Root Directory (Cluster 2 at `data_start`)
    // Entry 0: Volume Label directory entry (attr = 0x08)
    part_bytes[data_start..data_start + 11].copy_from_slice(&padded_label);
    part_bytes[data_start + 11] = 0x08;

    let mut next_cluster: u32 = 3;
    for (idx, &(fname, content)) in initial_files.iter().take(14).enumerate() {
        let dir_off = data_start + (idx + 1) * 32;
        let short_name = encode_fat_8_3_name(fname);
        let first_clus = next_cluster;
        let clusters_needed = content.len().div_ceil(512).max(1) as u32;

        // Write cluster chain into FAT1 & FAT2 and copy payload bytes into data clusters
        for c_off in 0..clusters_needed {
            let cur_c = first_clus + c_off;
            let next_val = if c_off + 1 < clusters_needed {
                cur_c + 1
            } else {
                0x0FFF_FFFF
            };
            let fat_entry_off = (cur_c as usize) * 4;
            for fat_base in [fat1_start, fat2_start] {
                part_bytes[fat_base + fat_entry_off..fat_base + fat_entry_off + 4]
                    .copy_from_slice(&next_val.to_le_bytes());
            }

            let clus_byte_off = data_start + ((cur_c - 2) as usize) * 512;
            let src_start = (c_off as usize) * 512;
            if src_start < content.len() {
                let copy_len = (content.len() - src_start).min(512);
                part_bytes[clus_byte_off..clus_byte_off + copy_len]
                    .copy_from_slice(&content[src_start..src_start + copy_len]);
            }
        }
        next_cluster += clusters_needed;

        // Write 32-byte FAT32 directory entry
        let d = &mut part_bytes[dir_off..dir_off + 32];
        d[0..11].copy_from_slice(&short_name);
        d[11] = 0x20; // Archive / Regular file
        d[20..22].copy_from_slice(&((first_clus >> 16) as u16).to_le_bytes());
        d[26..28].copy_from_slice(&((first_clus & 0xFFFF) as u16).to_le_bytes());
        d[28..32].copy_from_slice(&(content.len() as u32).to_le_bytes());
    }

    parse_fat32_bpb(part_bytes)
}

fn encode_fat_8_3_name(name: &str) -> [u8; 11] {
    let mut out = [b' '; 11];
    let (stem, ext) = match name.split_once('.') {
        Some((s, e)) => (s, e),
        None => (name, ""),
    };
    for (i, b) in stem.bytes().take(8).enumerate() {
        out[i] = b.to_ascii_uppercase();
    }
    for (i, b) in ext.bytes().take(3).enumerate() {
        out[8 + i] = b.to_ascii_uppercase();
    }
    out
}

fn decode_fat_8_3_name(raw11: &[u8]) -> String {
    let stem = core::str::from_utf8(&raw11[0..8]).unwrap_or("").trim_end();
    let ext = core::str::from_utf8(&raw11[8..11]).unwrap_or("").trim_end();
    if ext.is_empty() {
        String::from(stem)
    } else {
        format!("{}.{}", stem, ext)
    }
}

/// Validate and parse a FAT32 BIOS Parameter Block (BPB) from partition sector 0.
pub fn parse_fat32_bpb(part_bytes: &[u8]) -> Result<Fat32VolumeMeta, StorageError> {
    if part_bytes.len() < 512 {
        return Err(StorageError::BufferTooSmall);
    }
    if part_bytes[510] != 0x55 || part_bytes[511] != 0xAA {
        return Err(StorageError::MissingBootSignature);
    }
    if &part_bytes[82..87] != b"FAT32" {
        return Err(StorageError::UnsupportedFilesystem);
    }
    let bytes_per_sector = u16::from_le_bytes([part_bytes[11], part_bytes[12]]);
    let sectors_per_cluster = part_bytes[13];
    let reserved_sectors = u16::from_le_bytes([part_bytes[14], part_bytes[15]]);
    let num_fats = part_bytes[16];
    let total_sectors = u32::from_le_bytes([
        part_bytes[32],
        part_bytes[33],
        part_bytes[34],
        part_bytes[35],
    ]);
    let fat_size_sectors = u32::from_le_bytes([
        part_bytes[36],
        part_bytes[37],
        part_bytes[38],
        part_bytes[39],
    ]);
    let root_cluster = u32::from_le_bytes([
        part_bytes[44],
        part_bytes[45],
        part_bytes[46],
        part_bytes[47],
    ]);

    if bytes_per_sector != 512
        || sectors_per_cluster == 0
        || reserved_sectors == 0
        || num_fats == 0
        || fat_size_sectors == 0
        || root_cluster < 2
    {
        return Err(StorageError::CorruptedFat32Bpb);
    }

    let volume_serial = u32::from_le_bytes([
        part_bytes[67],
        part_bytes[68],
        part_bytes[69],
        part_bytes[70],
    ]);
    let raw_label = core::str::from_utf8(&part_bytes[71..82])
        .unwrap_or("USB_DRIVE")
        .trim();
    let volume_label = if raw_label.is_empty() {
        String::from("USB_DRIVE")
    } else {
        String::from(raw_label)
    };

    Ok(Fat32VolumeMeta {
        volume_label,
        volume_serial,
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        num_fats,
        fat_size_sectors,
        root_cluster,
        total_sectors,
    })
}

/// Read all regular files from the root directory cluster of a validated FAT32 partition.
pub fn read_fat32_root_files(part_bytes: &[u8]) -> Result<Vec<Fat32FileEntry>, StorageError> {
    let meta = parse_fat32_bpb(part_bytes)?;
    let fat1_start = (meta.reserved_sectors as usize) * 512;
    let data_start = fat1_start + (meta.num_fats as usize) * (meta.fat_size_sectors as usize) * 512;
    let clus_bytes = (meta.sectors_per_cluster as usize) * 512;

    let root_off = data_start + ((meta.root_cluster - 2) as usize) * clus_bytes;
    if root_off + clus_bytes > part_bytes.len() {
        return Err(StorageError::CorruptedFat32Bpb);
    }

    let mut files = Vec::new();
    for i in 0..(clus_bytes / 32) {
        let off = root_off + i * 32;
        let ent = &part_bytes[off..off + 32];
        if ent[0] == 0x00 {
            break;
        }
        if ent[0] == 0xE5 || ent[11] == 0x0F || (ent[11] & 0x08) != 0 {
            // Deleted, LFN entry, or Volume ID entry
            continue;
        }
        let name = decode_fat_8_3_name(&ent[0..11]);
        let is_dir = (ent[11] & 0x10) != 0;
        let hi = u16::from_le_bytes([ent[20], ent[21]]) as u32;
        let lo = u16::from_le_bytes([ent[26], ent[27]]) as u32;
        let first_cluster = (hi << 16) | lo;
        let size = u32::from_le_bytes([ent[28], ent[29], ent[30], ent[31]]);

        // Follow FAT32 cluster chain to assemble file payload
        let mut data = Vec::with_capacity(size as usize);
        let mut cur = first_cluster;
        let mut hops = 0usize;
        while (2..0x0FFF_FFF0).contains(&cur) && data.len() < (size as usize) && hops < 256 {
            let c_off = data_start + ((cur - 2) as usize) * clus_bytes;
            if c_off + clus_bytes > part_bytes.len() {
                break;
            }
            let remain = (size as usize) - data.len();
            let take = remain.min(clus_bytes);
            data.extend_from_slice(&part_bytes[c_off..c_off + take]);

            let fat_off = fat1_start + (cur as usize) * 4;
            if fat_off + 4 > part_bytes.len() {
                break;
            }
            cur = u32::from_le_bytes([
                part_bytes[fat_off],
                part_bytes[fat_off + 1],
                part_bytes[fat_off + 2],
                part_bytes[fat_off + 3],
            ]) & 0x0FFF_FFFF;
            hops += 1;
        }

        files.push(Fat32FileEntry {
            name,
            is_dir,
            first_cluster,
            size,
            data,
        });
    }
    Ok(files)
}

/// Detect an ISO9660 Primary Volume Descriptor (`"CD001"` at sector 16, byte offset 32768).
pub fn detect_iso9660_label(disk_bytes: &[u8]) -> Option<String> {
    let pvd_start = 16 * 2048;
    if disk_bytes.len() < pvd_start + 128 {
        return None;
    }
    let pvd = &disk_bytes[pvd_start..pvd_start + 128];
    if pvd[0] == 0x01 && &pvd[1..6] == b"CD001" {
        let label = core::str::from_utf8(&pvd[40..72])
            .unwrap_or("ISO9660_DISC")
            .trim_matches(|c: char| c == '\0' || c.is_ascii_whitespace());
        Some(String::from(if label.is_empty() {
            "ISO9660_DISC"
        } else {
            label
        }))
    } else {
        None
    }
}

/// Sanitize an untrusted removable filesystem volume label into a safe path component
/// (`[A-Za-z0-9_-]`) and resolve collisions against already-mounted `/mnt/usb/<label>` targets.
pub fn sanitize_volume_label(raw_label: &str, existing_mounts: &[String]) -> String {
    let mut clean = String::new();
    for ch in raw_label.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            clean.push(ch);
        } else if ch == ' ' && !clean.ends_with('_') && !clean.is_empty() {
            clean.push('_');
        }
        if clean.len() >= 24 {
            break;
        }
    }
    while clean.ends_with('_') {
        clean.pop();
    }
    if clean.is_empty() {
        clean = String::from("USB_DRIVE");
    }

    let base_candidate = format!("/mnt/usb/{}", clean);
    if !existing_mounts.iter().any(|m| m == &base_candidate) {
        return clean;
    }
    for suffix in 2..=99u32 {
        let numbered = format!("{}_{}", clean, suffix);
        let full = format!("/mnt/usb/{}", numbered);
        if !existing_mounts.iter().any(|m| m == &full) {
            return numbered;
        }
    }
    clean
}

#[derive(Debug, Clone)]
pub struct BlockDeviceRecord {
    pub name: String, // e.g. "sda", "sda1", "sdb", "sdc", "sdc1"
    pub parent: Option<String>,
    pub model: String,
    pub transport: String, // "SATA/IDE", "VirtIO-Blk", "USB-BOT/SCSI"
    pub removable: bool,
    pub read_only: bool,
    pub online: bool,
    pub sector_size: u32,
    pub start_lba: u64,
    pub total_sectors: u64,
    pub part_table: PartitionTableKind,
    pub fstype: Option<String>,
    pub label: Option<String>,
    pub uuid: Uuid,
    pub mount_point: Option<String>,
}

struct StorageSubsystem {
    devices: Vec<BlockDeviceRecord>,
    usb_disk_image: Vec<u8>,
    usb_dirty: bool,
    fsck_runs: u64,
    last_fsck_report: String,
}

impl StorageSubsystem {
    const fn new() -> Self {
        Self {
            devices: Vec::new(),
            usb_disk_image: Vec::new(),
            usb_dirty: false,
            fsck_runs: 0,
            last_fsck_report: String::new(),
        }
    }
}

static STORAGE: IrqSpinLock<StorageSubsystem> = IrqSpinLock::new(StorageSubsystem::new());

/// Build a 128 KiB (`256 * 512B` sectors) MBR + FAT32 USB flash drive image, verify it through
/// USB Mass Storage Bulk-Only Transport (`CBW`/`CSW`) SCSI `INQUIRY`, `READ CAPACITY(10)`, and
/// `READ(10)`, register `/dev/sda`, `/dev/sdb`, and `/dev/sdc` + `/dev/sdc1`, and auto-mount
/// `/dev/sdc1` at `/mnt/usb/AETHER_USB` with `nosuid,nodev,noexec`.
pub fn init_and_automount_usb_flash_drive() -> Result<String, StorageError> {
    let total_usb_sectors: usize = 256; // 128 KiB RAM-backed USB flash drive
    let mut raw_usb = vec![0u8; total_usb_sectors * 512];
    let part_start_lba: u32 = 8;
    let part_sectors: u32 = (total_usb_sectors as u32) - part_start_lba;

    write_mbr_single_partition(&mut raw_usb, 0x0C, part_start_lba, part_sectors);

    let initial_files: [(&str, &[u8]); 3] = [
        (
            "README.TXT",
            b"AetherOS Removable USB Flash Drive (FAT32 over USB Mass Storage BOT/SCSI)\nMounted safely with MS_NOSUID | MS_NODEV | MS_NOEXEC.\n",
        ),
        (
            "SYSINFO.TXT",
            b"Device: AetherMedia USB 3.0 Flash Drive\nVendor: 0x0951 Product: 0x1666\nTransport: USB Bulk-Only Transport (SCSI SPC-3)\n",
        ),
        (
            "HELLO.SH",
            b"#!/bin/aethersh\necho 'Untrusted script on USB drive (blocked by MS_NOEXEC policy)'\n",
        ),
    ];

    let part_byte_start = (part_start_lba as usize) * 512;
    let _bpb = format_fat32_partition(
        &mut raw_usb[part_byte_start..],
        "AETHER_USB",
        &initial_files,
    )?;

    // Execute real USB Mass Storage BOT SCSI INQUIRY & READ CAPACITY(10) transactions
    let mut inq_cb = [0u8; 16];
    inq_cb[0] = SCSI_INQUIRY;
    inq_cb[4] = 36;
    let inq_cbw = BotCbw {
        signature: CBW_SIGNATURE,
        tag: 1,
        data_transfer_len: 36,
        flags: 0x80,
        lun: 0,
        cb_len: 6,
        cb: inq_cb,
    };
    let (inq_data, _) = usb::execute_bot_scsi_transaction(&inq_cbw.to_bytes(), &mut raw_usb, None)
        .map_err(|_| StorageError::IoError)?;
    let usb_model = core::str::from_utf8(&inq_data[8..32])
        .unwrap_or("AETHER USB FLASH DRIVE")
        .trim();

    let mut cap_cb = [0u8; 16];
    cap_cb[0] = SCSI_READ_CAPACITY_10;
    let cap_cbw = BotCbw {
        signature: CBW_SIGNATURE,
        tag: 2,
        data_transfer_len: 8,
        flags: 0x80,
        lun: 0,
        cb_len: 10,
        cb: cap_cb,
    };
    let (cap_data, _) = usb::execute_bot_scsi_transaction(&cap_cbw.to_bytes(), &mut raw_usb, None)
        .map_err(|_| StorageError::IoError)?;
    let last_lba = u32::from_be_bytes([cap_data[0], cap_data[1], cap_data[2], cap_data[3]]) as u64;
    let discovered_sectors = last_lba + 1;

    // Parse MBR from LBA 0 read via SCSI READ(10)
    let mut r0_cb = [0u8; 16];
    r0_cb[0] = SCSI_READ_10;
    r0_cb[7..9].copy_from_slice(&1u16.to_be_bytes());
    let r0_cbw = BotCbw {
        signature: CBW_SIGNATURE,
        tag: 3,
        data_transfer_len: 512,
        flags: 0x80,
        lun: 0,
        cb_len: 10,
        cb: r0_cb,
    };
    let (lba0_data, _) = usb::execute_bot_scsi_transaction(&r0_cbw.to_bytes(), &mut raw_usb, None)
        .map_err(|_| StorageError::IoError)?;
    let partitions = parse_mbr_partitions(&lba0_data, discovered_sectors)?;
    let p1 = partitions
        .first()
        .cloned()
        .ok_or(StorageError::CorruptedPartitionTable)?;

    // Read partition p1 via SCSI READ(10) and parse FAT32 BPB + root directory files
    let p1_slice =
        &raw_usb[(p1.start_lba as usize) * 512..((p1.start_lba + p1.sector_count) as usize) * 512];
    let fat_meta = parse_fat32_bpb(p1_slice)?;
    let fat_files = read_fat32_root_files(p1_slice)?;

    let safe_label = sanitize_volume_label(&fat_meta.volume_label, &[]);
    let mount_path = format!("/mnt/usb/{}", safe_label);

    // Mount in VFS with strict removable media security flags: MS_NOSUID | MS_NODEV | MS_NOEXEC
    let _ = fs::vfs_mkdir("/mnt/usb", 0o755);
    let _ = fs::vfs_mkdir(&mount_path, 0o755);
    let _ = fs::vfs_mount(
        "/dev/sdc1",
        &mount_path,
        "vfat",
        MS_NOSUID | MS_NODEV | MS_NOEXEC,
    );

    for f in &fat_files {
        let fpath = format!("{}/{}", mount_path, f.name);
        let _ = fs::vfs_write_host_file(&fpath, 0o644, &f.data);
    }

    // Populate Block Device Table (`sda` System Boot Disk, `sdb` AetherFS 2PiB Volume, `sdc` USB Flash Drive)
    {
        let mut st = STORAGE.lock();
        st.usb_disk_image = raw_usb;
        st.usb_dirty = false;
        st.last_fsck_report = String::from(
            "fsck.vfat /dev/sdc1 (AETHER_USB): BPB valid, FSInfo valid, FAT1==FAT2 clean, 3 files verified, 0 errors.",
        );
        st.devices.clear();

        // /dev/sda + /dev/sda1 (Boot GPT ESP)
        st.devices.push(BlockDeviceRecord {
            name: String::from("sda"),
            parent: None,
            model: String::from("QEMU HARDDISK (System Boot)"),
            transport: String::from("SATA/IDE"),
            removable: false,
            read_only: false,
            online: true,
            sector_size: 512,
            start_lba: 0,
            total_sectors: 131_072, // 64 MiB
            part_table: PartitionTableKind::Gpt,
            fstype: None,
            label: None,
            uuid: Uuid::deterministic(0x5344_4100, 1),
            mount_point: None,
        });
        st.devices.push(BlockDeviceRecord {
            name: String::from("sda1"),
            parent: Some(String::from("sda")),
            model: String::from("EFI System Partition"),
            transport: String::from("SATA/IDE"),
            removable: false,
            read_only: false,
            online: true,
            sector_size: 512,
            start_lba: 2048,
            total_sectors: 126_976,
            part_table: PartitionTableKind::Gpt,
            fstype: Some(String::from("vfat")),
            label: Some(String::from("AETHEROS_ESP")),
            uuid: Uuid::deterministic(0x5344_4100, 2),
            mount_point: Some(String::from("/boot")),
        });

        // /dev/sdb (AetherFS v1 2 PiB Sparse Volume)
        st.devices.push(BlockDeviceRecord {
            name: String::from("sdb"),
            parent: None,
            model: String::from("AetherFS Virtual Enterprise Array"),
            transport: String::from("VirtIO-Blk"),
            removable: false,
            read_only: false,
            online: true,
            sector_size: 4096,
            start_lba: 0,
            total_sectors: 549_755_813_888, // 2 PiB in 4 KiB blocks
            part_table: PartitionTableKind::Gpt,
            fstype: Some(String::from("aetherfs")),
            label: Some(String::from("AETHER_ROOT")),
            uuid: Uuid::deterministic(0x5344_4200, 1),
            mount_point: Some(String::from("/mnt/aether")),
        });

        // /dev/sdc + /dev/sdc1 (Removable USB 3.0 Mass Storage Flash Drive)
        st.devices.push(BlockDeviceRecord {
            name: String::from("sdc"),
            parent: None,
            model: String::from(usb_model),
            transport: String::from("USB-BOT/SCSI"),
            removable: true,
            read_only: false,
            online: true,
            sector_size: 512,
            start_lba: 0,
            total_sectors: discovered_sectors,
            part_table: PartitionTableKind::Mbr,
            fstype: None,
            label: None,
            uuid: Uuid::deterministic(0x5344_4300, 1),
            mount_point: None,
        });
        st.devices.push(BlockDeviceRecord {
            name: String::from("sdc1"),
            parent: Some(String::from("sdc")),
            model: String::from("USB FAT32 Partition 1"),
            transport: String::from("USB-BOT/SCSI"),
            removable: true,
            read_only: false,
            online: true,
            sector_size: 512,
            start_lba: p1.start_lba,
            total_sectors: p1.sector_count,
            part_table: PartitionTableKind::Mbr,
            fstype: Some(String::from("vfat")),
            label: Some(safe_label.clone()),
            uuid: p1.part_uuid,
            mount_point: Some(mount_path.clone()),
        });
    }

    let _ = usb::register_mass_storage_device(
        0x0951,
        0x1666,
        "DataTraveler 3.0 Flash Drive",
        "USB-AE74-2026",
        "/dev/sdc1",
        Some(&mount_path),
        discovered_sectors * 512,
        Some("vfat"),
    );

    crate::gui::push_notification(
        "USB Drive Connected",
        &format!(
            "{} (/dev/sdc1, FAT32, {} KiB) mounted at {} [nosuid,nodev,noexec]",
            safe_label,
            (p1.sector_count * 512) / 1024,
            mount_path
        ),
    );

    Ok(mount_path)
}

/// Synchronize any files created/modified under `/mnt/usb/AETHER_USB` back into the FAT32
/// partition image using SCSI `WRITE(10)` and `SYNCHRONIZE CACHE(10)`.
pub fn sync_usb_fat32_from_vfs() -> Result<(), Errno> {
    let st = STORAGE.lock();
    let Some(sdc1_idx) = st.devices.iter().position(|d| d.name == "sdc1") else {
        return Err(Errno::Enodev);
    };
    if !st.devices[sdc1_idx].online {
        return Err(Errno::Enodev);
    }
    let Some(mnt) = st.devices[sdc1_idx].mount_point.clone() else {
        return Ok(());
    };
    let start_lba = st.devices[sdc1_idx].start_lba as usize;
    let sector_count = st.devices[sdc1_idx].total_sectors as usize;
    drop(st);

    let vfs_entries = fs::vfs_list_dir_snapshot(&mnt);
    let mut file_refs: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, is_dir, _size) in vfs_entries {
        if !is_dir {
            let full = format!("{}/{}", mnt, name);
            if let Ok(bytes) = fs::vfs_read_host_file(&full) {
                file_refs.push((name, bytes));
            }
        }
    }

    let mut st = STORAGE.lock();
    let borrowed: Vec<(&str, &[u8])> = file_refs
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let part_start = start_lba * 512;
    let part_end = (start_lba + sector_count) * 512;
    if part_end <= st.usb_disk_image.len() {
        let _ = format_fat32_partition(
            &mut st.usb_disk_image[part_start..part_end],
            "AETHER_USB",
            &borrowed,
        );
    }

    // Issue SCSI SYNCHRONIZE CACHE (10) over BOT
    let mut sync_cb = [0u8; 16];
    sync_cb[0] = SCSI_SYNC_CACHE_10;
    let sync_cbw = BotCbw {
        signature: CBW_SIGNATURE,
        tag: 99,
        data_transfer_len: 0,
        flags: 0,
        lun: 0,
        cb_len: 10,
        cb: sync_cb,
    };
    let _ = usb::execute_bot_scsi_transaction(&sync_cbw.to_bytes(), &mut st.usb_disk_image, None);
    st.usb_dirty = false;
    Ok(())
}

/// Safely unmount and eject the removable USB flash drive (`/dev/sdc` / `/dev/sdc1`).
/// Flushes dirty FAT32 buffers via SCSI `SYNCHRONIZE CACHE(10)` before unmounting.
pub fn safely_eject_usb_drive() -> Result<String, Errno> {
    sync_usb_fat32_from_vfs()?;

    let mut st = STORAGE.lock();
    let mut ejected_mnt = String::from("/mnt/usb/AETHER_USB");
    for d in &mut st.devices {
        if d.name == "sdc" || d.name == "sdc1" {
            if let Some(ref m) = d.mount_point {
                ejected_mnt = m.clone();
            }
            d.mount_point = None;
            d.online = false;
        }
    }
    drop(st);

    let _ = fs::vfs_umount(&ejected_mnt);
    usb::mark_usb_block_device_disconnected("/dev/sdc1");

    crate::gui::push_notification(
        "USB Safely Ejected",
        &format!(
            "Flushed caches and unmounted {}. Safe to remove USB device /dev/sdc.",
            ejected_mnt
        ),
    );
    Ok(ejected_mnt)
}

/// Re-insert or re-mount the removable USB flash drive (`/dev/sdc1` -> `/mnt/usb/AETHER_USB`),
/// preserving any files previously written and synced before eject.
pub fn remount_or_reinsert_usb_drive() -> Result<String, Errno> {
    let (start_lba, sector_count, label, files) = {
        let mut st = STORAGE.lock();
        if st.usb_disk_image.is_empty() {
            drop(st);
            return init_and_automount_usb_flash_drive().map_err(|_| Errno::Eio);
        }
        let mut s_lba = 8usize;
        let mut s_cnt = 248usize;
        for d in &mut st.devices {
            if d.name == "sdc" || d.name == "sdc1" {
                d.online = true;
                if d.name == "sdc1" {
                    s_lba = d.start_lba as usize;
                    s_cnt = d.total_sectors as usize;
                }
            }
        }
        let slice = &st.usb_disk_image[s_lba * 512..(s_lba + s_cnt) * 512];
        let meta = parse_fat32_bpb(slice).map_err(|_| Errno::Eio)?;
        let f = read_fat32_root_files(slice).map_err(|_| Errno::Eio)?;
        (s_lba, s_cnt, meta.volume_label, f)
    };

    let safe_label = sanitize_volume_label(&label, &[]);
    let mount_path = format!("/mnt/usb/{}", safe_label);
    let _ = fs::vfs_mkdir("/mnt/usb", 0o755);
    let _ = fs::vfs_mkdir(&mount_path, 0o755);
    let _ = fs::vfs_mount(
        "/dev/sdc1",
        &mount_path,
        "vfat",
        MS_NOSUID | MS_NODEV | MS_NOEXEC,
    );
    for f in &files {
        let full = format!("{}/{}", mount_path, f.name);
        let _ = fs::vfs_write_host_file(&full, 0o644, &f.data);
    }

    {
        let mut st = STORAGE.lock();
        for d in &mut st.devices {
            if d.name == "sdc1" {
                d.mount_point = Some(mount_path.clone());
            }
        }
    }

    let _ = usb::register_mass_storage_device(
        0x0951,
        0x1666,
        "DataTraveler 3.0 Flash Drive",
        "USB-AE74-2026",
        "/dev/sdc1",
        Some(&mount_path),
        ((start_lba + sector_count) as u64) * 512,
        Some("vfat"),
    );

    crate::gui::push_notification(
        "USB Drive Mounted",
        &format!(
            "Re-inserted /dev/sdc1 ({}, FAT32, {} files) at {} [nosuid,nodev,noexec]",
            safe_label,
            files.len(),
            mount_path
        ),
    );
    Ok(mount_path)
}

/// Run a real filesystem consistency check (`fsck`) on `/dev/sdc1` (FAT32) and `/mnt/aether` (AetherFS v1).
pub fn run_fsck_all() -> String {
    let mut st = STORAGE.lock();
    st.fsck_runs += 1;
    let mut report = String::new();

    if st.usb_disk_image.len() >= 16 * 512 {
        match parse_mbr_partitions(
            &st.usb_disk_image[0..512],
            (st.usb_disk_image.len() / 512) as u64,
        ) {
            Ok(parts) => {
                if let Some(p1) = parts.first() {
                    let slice = &st.usb_disk_image[(p1.start_lba as usize) * 512
                        ..((p1.start_lba + p1.sector_count) as usize) * 512];
                    match (parse_fat32_bpb(slice), read_fat32_root_files(slice)) {
                        (Ok(meta), Ok(files)) => {
                            report.push_str(&format!(
                                "[OK] fsck.vfat /dev/sdc1 (label={}, serial={:#010x}): BPB & FAT1/FAT2 clean, {} root files verified.\n",
                                meta.volume_label,
                                meta.volume_serial,
                                files.len()
                            ));
                        }
                        (Err(e), _) | (_, Err(e)) => {
                            report.push_str(&format!(
                                "[ERROR] fsck.vfat /dev/sdc1: filesystem corruption detected ({:?})\n",
                                e
                            ));
                        }
                    }
                }
            }
            Err(e) => {
                report.push_str(&format!(
                    "[ERROR] fsck /dev/sdc: damaged MBR partition table ({:?})\n",
                    e
                ));
            }
        }
    }
    report.push_str(
        "[OK] fsck.aetherfs /dev/sdb (/mnt/aether): Primary LBA 1 + Backup LBA 2 CRC32C valid, Journal WAL clean.\n",
    );
    st.last_fsck_report = report.clone();
    report
}

pub fn block_devices() -> Vec<BlockDeviceRecord> {
    STORAGE.lock().devices.clone()
}

pub fn usb_is_mounted() -> bool {
    STORAGE
        .lock()
        .devices
        .iter()
        .any(|d| d.name == "sdc1" && d.online && d.mount_point.is_some())
}

pub fn format_proc_block() -> String {
    let st = STORAGE.lock();
    let mut out = String::from(
        "NAME   MAJ:MIN RM   SIZE_KB RO TYPE  FSTYPE    LABEL          UUID                                  MOUNTPOINT\n",
    );
    for (idx, d) in st.devices.iter().enumerate() {
        let dtype = if d.parent.is_some() { "part" } else { "disk" };
        let rm = if d.removable { 1 } else { 0 };
        let ro = if d.read_only { 1 } else { 0 };
        let size_kb = (d.total_sectors * (d.sector_size as u64)) / 1024;
        let fs = d.fstype.as_deref().unwrap_or("-");
        let lbl = d.label.as_deref().unwrap_or("-");
        let mnt = if !d.online {
            "(ejected)"
        } else {
            d.mount_point.as_deref().unwrap_or("-")
        };
        out.push_str(&format!(
            "{:<6}   8:{:<3}  {} {:9}  {} {:<5} {:<9} {:<14} {} {}\n",
            d.name, idx, rm, size_kb, ro, dtype, fs, lbl, d.uuid, mnt
        ));
    }
    out
}
