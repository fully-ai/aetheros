# AetherOS Filesystem Support Matrix (`FILESYSTEM_SUPPORT.md`)

| Filesystem | Driver Location | Read / Write | Security & Integrity Features | Primary Use Case |
| :--- | :--- | :--- | :--- | :--- |
| **AetherFS v1** | `crates/aether-fs-core/`, `kernel/src/fs/aetherfs.rs` | Read / Write | `>= 1 PiB` 64-bit extents, Primary LBA 1 + Backup LBA 2 superblocks, Castagnoli CRC32C, `AETHJRNL` WAL journal, 128-bit UUID ownership, UUID ACLs, `execute_as_uuid` | Native persistent OS storage (`/mnt/aether`) |
| **FAT32 (`vfat`)** | `kernel/src/storage/mod.rs` | Read / Write | BPB & FSInfo validation, dual FAT1/FAT2 cluster chain updates, SCSI `SYNCHRONIZE CACHE(10)` on eject, mounted with `MS_NOSUID \| MS_NODEV \| MS_NOEXEC` | UEFI ESP (`/boot`) & Removable USB flash drives (`/mnt/usb/<LABEL>`) |
| **ISO9660 (`iso9660`)** | `kernel/src/storage/mod.rs` | Read-Only Detection | Primary Volume Descriptor (`CD001` at LBA 16) validation and volume label extraction | Optical / Live CD media recognition |
| **RamFS / TmpFS** | `kernel/src/fs/mod.rs` | Read / Write | Full UUID & ACL metadata, sticky bit (`01777`) on `/tmp`, `MS_NOSUID` on `/tmp` and `/run` | Root initramfs (`/`), `/tmp`, `/run` |
| **DevFS & ProcFS** | `kernel/src/fs/mod.rs` | Character I/O & Read-Only | Mounted with `MS_NOSUID \| MS_NOEXEC` | `/dev/*` device nodes & `/proc/*` live kernel telemetry |
