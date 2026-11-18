# ADR-0005: AetherFS On-Disk Design & UUID-Based Identity Model

- **Status**: Accepted
- **Date**: 2026-11-18
- **Decision Makers**: AetherOS Core Engineering

## Context

AetherOS requires an independent, crash-consistent native filesystem (`AetherFS`) with:
- Minimum designed volume capacity of **1 PiB** (`2^50` bytes) using 64-bit block/byte addressing and checked arithmetic.
- Protected inode metadata embedding **UUID-based user/group ownership**, POSIX-style modes, **UUID-based ACLs**, and kernel-enforced **`execute_as_uuid`** execution identity policy.
- Checksummed metadata and write-ahead journaling for crash recovery.

## Decision

### 1. AetherFS v1 Layout (Little-Endian, Explicit Versioning)
- **Block Size**: 4096 bytes (`4 KiB`) default (configurable 4 KiB..64 KiB). At 4 KiB blocks, 64-bit block indices address up to `2^64 * 4096` bytes (`64 ZiB`), easily surpassing 1 PiB (`2^38` 4 KiB blocks = `274,877,906,944` blocks) without 32-bit overflow.
- **Superblock**: Primary at byte offset `4096` (block 1) and backup copy at block 2 (and secondary mirror), carrying magic `b"AETHERFS"`, format version, volume UUID, block size, total blocks (`u64`), inode table geometry, extent tree geometry, journal region (`start_block`, `block_count`), feature flags, and **CRC32C checksum**.
- **Protected Object Record (Inode, 256 bytes, CRC32C checksummed)**:
  - `inode_id: u64`
  - `object_uuid: [u8; 16]`
  - `owner_user_uuid: [u8; 16]`
  - `owner_group_uuid: [u8; 16]`
  - `execute_as_uuid: [u8; 16]` (all-zero = none / run as caller; non-zero = explicit kernel-validated execution identity)
  - `mode: u32` (type + rwx + suid/sgid/sticky)
  - `flags: u32` (immutable, append-only, exec_as_enabled, sparse, ...)
  - `size: u64` (64-bit byte size, sparse-file capable)
  - `nlink: u32`
  - `acl_block: u64` (optional external ACL block)
  - `ctime_ns / mtime_ns / atime_ns: u64`
  - Inline extent array (direct extents) + indirect extent tree root block (`extent_root: u64`) for huge/sparse files.
  - `checksum: u32` (CRC32C over inode record)
- **Write-Ahead Metadata Journal**:
  - Ring-buffered journal transactions (`TX_BEGIN`, `TX_WRITE_BLOCK`, `TX_COMMIT`) with monotonic sequence numbers and CRC32C checksums.
  - Recovery replays only complete, checksum-verified committed transactions and discards torn/uncommitted tail transactions.

### 2. UUID Identity & `execute_as_uuid` Security Model
- Reserved Root User UUID: `00000000-0000-4000-8000-000000000001` (UID 0).
- Reserved Root Group UUID: `00000000-0000-4000-8000-000000000000` (GID 0).
- Normal/system users receive stable RFC 4122 UUIDs mapped bijectively to numeric UID/GID for POSIX ergonomics.
- `execute_as_uuid` may ONLY be modified by root or a process holding `CAP_SET_EXECUTE_AS`.
- On `execve`: if `execute_as_uuid` is set (and mount is not `nosuid`, and process does not have `no_new_privs`), the kernel validates that the target UUID exists in the identity database (an invalid/deleted UUID fails with `EACCES` and NEVER falls back to root), checks execution policy, audit-logs the transition, and updates effective credentials.

## Consequences

- All arithmetic uses `checked_add` / `checked_mul` to guarantee 1 PiB boundary safety.
- Shared `aether-fs-core` crate allows exhaustive host-side 1 PiB sparse metadata tests, power-loss torn-write simulations, and corruption fuzzing alongside direct in-kernel execution.
