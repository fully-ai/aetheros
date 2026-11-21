# AetherOS USB Mass Storage & Removable Media (`USB_STORAGE.md`)

## 1. Bulk-Only Transport (BOT) & SCSI Command Engine (`kernel/src/usb/mod.rs`)

AetherOS implements the USB Mass Storage Bulk-Only Transport specification:
- **Command Block Wrapper (`BotCbw`, 31 bytes)**: Little-endian signature `0x43425355` (`"USBC"`), transaction `tag`, `data_transfer_len`, direction `flags`, `lun`, and 16-byte SCSI Command Descriptor Block (`cb`).
- **Command Status Wrapper (`BotCsw`, 13 bytes)**: Little-endian signature `0x53425355` (`"USBS"`), matching `tag`, `data_residue`, and `status` (`0 = Command Passed`, `1 = Command Failed`).
- **Implemented SCSI Commands**:
  - `TEST UNIT READY (0x00)`
  - `REQUEST SENSE (0x03)`
  - `INQUIRY (0x12)`
  - `READ CAPACITY (10) (0x25)`
  - `READ (10) (0x28)` (with checked 64-bit LBA bounds verification)
  - `WRITE (10) (0x2A)` (with checked 64-bit LBA bounds verification)
  - `SYNCHRONIZE CACHE (10) (0x35)`

## 2. Automatic Mounting & Safe Eject Workflow (`kernel/src/storage/mod.rs`)

1. **Discovery**: When `/dev/sdc` (`0951:1666 DataTraveler 3.0 Flash Drive`) is attached, the kernel executes SCSI `INQUIRY`, `READ CAPACITY(10)`, and `READ(10)` over BOT.
2. **Partition & FAT32 Validation**: Parses the MBR/GPT partition table (`/dev/sdc1`), validates the FAT32 BIOS Parameter Block (BPB), and reads the FAT32 root directory cluster chain.
3. **Label Sanitization & Auto-Mount**: Sanitizes the volume label (`sanitize_volume_label`) to prevent path traversal and duplicate collisions, mounts `/dev/sdc1` at `/mnt/usb/AETHER_USB` with **`MS_NOSUID | MS_NODEV | MS_NOEXEC`**, and pushes a desktop notification.
4. **Safe Eject (`safely_eject_usb_drive`)**: Synchronizes VFS files back into the FAT32 cluster chain, issues SCSI `SYNCHRONIZE CACHE(10)`, unmounts `/mnt/usb/AETHER_USB`, marks `/dev/sdc` offline, and notifies the desktop.
