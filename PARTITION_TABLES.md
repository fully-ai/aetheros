# AetherOS Partition Table Support (`PARTITION_TABLES.md`)

## 1. Master Boot Record (MBR) Parser (`storage::parse_mbr_partitions`)
- Verifies boot signature `0x55AA` at offset `510..512`.
- Parses 4 primary 16-byte partition entries at offsets `446..510`.
- Validates active/inactive status flag (`0x00` or `0x80`), non-zero `start_lba`, and `start_lba.checked_add(sector_count) <= total_sectors`.
- Any partition extending beyond the physical disk size is rejected with `StorageError::PartitionOutOfBounds`.

## 2. GUID Partition Table (GPT) Parser (`storage::parse_gpt_partitions`)
- Verifies Protective MBR (`0xEE`) at LBA 0 and `"EFI PART"` signature at LBA 1 (`offset 512..520`).
- Validates `header_size` (`92..=512`), `my_lba == 1`, and `first_usable_lba <= last_usable_lba < total_sectors`.
- Parses 128-byte partition entries at `partition_entry_lba` (LBA 2+), extracting the 128-bit Partition Type GUID, 128-bit Unique Partition GUID (`part_uuid`), 64-bit `start_lba` and `end_lba`, and 36-character UTF-16LE partition name.
- Tested in `test_usb_mass_storage_partitions_fat32_and_eject` for both valid roundtrip and corrupted `end_lba` rejection.
