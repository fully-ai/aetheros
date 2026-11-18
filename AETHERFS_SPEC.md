# AetherFS v1 On-Disk Specification (`>= 1 PiB`)

## 1. Design Goals & Capacity Arithmetic (`crates/aether-fs-core/`)

**AetherFS v1** is AetherOS's native 64-bit extent-based journaling filesystem designed for large-scale capacity, crash resilience, and first-class UUID security metadata.

- **Block Size**: `4,096` bytes (`4 KiB`).
- **Block & Byte Addressing**: All block numbers (`total_blocks`, `start_block`, `logical_block`), file sizes (`size_bytes`), and allocation counters are **unsigned 64-bit integers (`u64`)**.
- **Petabyte+ Capacity**:
  - `1 PiB = 2^50 bytes = 1,125,899,906,842,624 bytes = 274,877,906,944` (`0x40_0000_0000`) 4 KiB blocks.
  - Because `u64::MAX` 4 KiB blocks spans `64 ZiB` (`2^76` bytes), AetherFS v1 natively formats, mounts, allocates, and addresses sparse and dense files well beyond **1 PiB** (verified in automated tests on a **2 PiB** volume with sparse file I/O at byte offset `1 PiB + 8 KiB`).

---

## 2. On-Disk Volume Layout

| Block Number (4 KiB LBA) | Region | Description |
| :--- | :--- | :--- |
| `LBA 0` | Boot / Reserved | Reserved for GPT / bootloader compatibility |
| `LBA 1` | **Primary Superblock** | Magic `AETHERFS` (`0x5346_5245_4854_4541`), version `1`, volume UUID, 64-bit geometry, CRC32C |
| `LBA 2` | **Backup Superblock** | Exact redundant replica of Primary Superblock for automatic failover on corruption |
| `LBA 3 .. 3+J-1` | **Write-Ahead Journal** | Header at `LBA 3` (`AETHJRNL` `0x4C4E_524A_4854_4541`) + descriptor & payload blocks |
| `LBA 3+J .. +B-1` | **Allocation Bitmap** | Block allocation bitmap for immediate region; higher petabyte extents allocated via 64-bit cursor |
| `LBA 3+J+B .. +I-1` | **Inode Table** | Array of 256-byte `AetherInode` structures (`16` inodes per 4 KiB block), each CRC32C-protected |
| `LBA 3+J+B+I .. +A-1` | **ACL Table** | Array of 24-byte `AetherAclEntry` records (`subject_uuid`, `allow_rwx`, `deny_rwx`) |
| `LBA DataStart .. TotalBlocks-1` | **64-Bit Data Arena** | File data extents and directory entry blocks spanning up to `>= 1 PiB` |

---

## 3. Metadata Checksumming (Castagnoli CRC32C)

Every critical on-disk structure embeds a 32-bit Castagnoli CRC32C checksum (`POLY = 0x82F6_3B78`) computed with the checksum field zeroed:
1. **`AetherSuperblock` (`checksum_crc32c`)**: Verified on `mount()`. If Primary Superblock (`LBA 1`) fails magic or CRC32C verification, `AetherFs::mount` automatically fails over to Backup Superblock (`LBA 2`), repairs `LBA 1`, and sets `recovered_from_backup_sb = true`.
2. **`AetherInode` (`256 bytes`, `checksum_crc32c` at offset `252..256`)**: Every inode read verifies its CRC32C before returning metadata or extents; tampering returns `FsError::CorruptedInodeChecksum(ino)`.
3. **`JournalHeader` (`header_crc32c` + `payload_crc32c`)**: Ensures incomplete or corrupted journal transactions are never replayed onto the volume.

---

## 4. Inode Format (`256 Bytes`, `#[repr(C)]`)

| Offset | Size | Field | Description |
| :--- | :--- | :--- | :--- |
| `0..8` | `u64` | `inode_id` | 64-bit Inode Number (`ROOT_INODE_ID = 1`) |
| `8..24` | `[u8; 16]` | `object_uuid` | Unique 128-bit UUID v4 of this filesystem object |
| `24..40` | `[u8; 16]` | `owner_user_uuid` | Owner User 128-bit UUID |
| `40..56` | `[u8; 16]` | `owner_group_uuid` | Owner Group 128-bit UUID |
| `56..72` | `[u8; 16]` | `execute_as_uuid` | Protected target execution identity UUID (`NIL` if disabled) |
| `72..88` | `4 * u32` | `uid`, `gid`, `mode`, `flags` | POSIX compatibility IDs, mode bits, and `INODE_FLAG_IMMUTABLE` / `INODE_FLAG_EXECUTE_AS` |
| `88..128` | `5 * u64` | `size_bytes`, `nlink`, `ctime_ns`, `mtime_ns`, `atime_ns` | 64-bit byte size, link count, and nanosecond timestamps |
| `128..144` | `2 * u64` | `acl_block`, `acl_count` | Pointer and count of UUID ACL entries |
| `144..240` | `4 * 24B` | `extents[4]` | Four inline 64-bit `AetherExtent { logical_block: u64, start_block: u64, block_count: u64 }` |
| `240..252` | `u64 + u32` | `indirect_extent_block`, `_reserved` | 64-bit block pointer for indirect extent tree |
| `252..256` | `u32` | `checksum_crc32c` | Castagnoli CRC32C over bytes `0..252` |

---

## 5. Write-Ahead Journal (`AETHJRNL`) & Crash Recovery

- **States**: `JOURNAL_CLEAN (0)`, `JOURNAL_PREPARING (1)`, `JOURNAL_COMMITTED (2)`.
- **Atomic Transaction Lifecycle**:
  1. `journal_stage_block(target_lba, block_data)` writes the new block into the journal payload region and marks the journal `JOURNAL_PREPARING`.
  2. `journal_commit()` computes `payload_crc32c` over all staged blocks and atomically writes the `JournalHeader` with `state = JOURNAL_COMMITTED`.
  3. On clean commit or subsequent `mount()`:
     - If `state == JOURNAL_COMMITTED` and both `header_crc32c` and `payload_crc32c` match, all staged blocks are replayed to their `target_lba` addresses and the journal is reset to `JOURNAL_CLEAN`.
     - If a crash occurred during `JOURNAL_PREPARING` (torn write), `mount()` rolls back and discards the uncommitted transaction, preserving on-disk consistency.
