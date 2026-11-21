# AetherOS USB & Removable Storage Status (`AETHEROS_USB_STATUS.md`)

- **Status**: **IMPLEMENTED & VERIFIED (`test_usb_mass_storage_partitions_fat32_and_eject` PASS in QEMU UEFI & BIOS)**
- **Host Controller Discovery**: PCI class `0x0C03` (`xHCI`, `EHCI`, `OHCI`, `UHCI`) enumerated in `kernel/src/usb/mod.rs` and exposed via `/proc/usb` and `/bin/lsusb`.
- **Descriptor Validation**: Strict bounds-checked parsing of Device (`0x01`), Configuration (`0x02`), Interface (`0x04`), and Endpoint (`0x05`) descriptors with malformed/out-of-bounds descriptor rejection.
- **USB Mass Storage BOT & SCSI**: Full Command Block Wrapper (`CBW` `"USBC"`) & Command Status Wrapper (`CSW` `"USBS"`) engine supporting `INQUIRY (0x12)`, `TEST UNIT READY (0x00)`, `REQUEST SENSE (0x03)`, `READ CAPACITY(10) (0x25)`, `READ(10) (0x28)`, `WRITE(10) (0x2A)`, and `SYNCHRONIZE CACHE(10) (0x35)`.
- **Partition & Filesystem Support**: MBR (`0x55AA`) and GPT (`"EFI PART"`) partition discovery with boundary validation; FAT32 read/write cluster chain support; ISO9660 (`CD001`) recognition; AetherFS v1 support.
- **Safe Automount & Eject**: Automounts `/dev/sdc1` at `/mnt/usb/AETHER_USB` with `MS_NOSUID | MS_NODEV | MS_NOEXEC`, sanitizes labels, syncs dirty files to FAT32 clusters on eject, and emits desktop notifications.
