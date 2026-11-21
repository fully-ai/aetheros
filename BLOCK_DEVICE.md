# AetherOS Block Device Layer (`BLOCK_DEVICE.md`)

## 1. Block Device Inventory (`kernel/src/storage/mod.rs`, `/proc/block`)

The block layer provides a unified registry (`BlockDeviceRecord`) for system disks, virtual storage arrays, and removable USB Mass Storage drives:

| Device | Parent | Transport | Sector Size | Capacity | Partition / FS | Mount Point |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `/dev/sda` | — | `SATA/IDE` | `512 B` | `64 MiB` | `GPT` | — |
| `/dev/sda1` | `sda` | `SATA/IDE` | `512 B` | `62 MiB` | `vfat` (`AETHEROS_ESP`) | `/boot` |
| `/dev/sdb` | — | `VirtIO-Blk` | `4096 B` | `2 PiB` | `aetherfs` (`AETHER_ROOT`) | `/mnt/aether` |
| `/dev/sdc` | — | `USB-BOT/SCSI` | `512 B` | `128 KiB` | `MBR` | — |
| `/dev/sdc1` | `sdc` | `USB-BOT/SCSI` | `512 B` | `124 KiB` | `vfat` (`AETHER_USB`) | `/mnt/usb/AETHER_USB` |

## 2. User-Space Inspection
- CLI: `lsblk`, `blkid`, `findmnt`, `df`, `fsck`, `eject`, `sync`.
- GUI: **Aether Disks & USB Partition Utility** (`AppKind::Disks`).
