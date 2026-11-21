# Aether Files — Native Graphical File Explorer (`FILE_MANAGER.md`)

## 1. Overview & Live VFS/USB Integration

**Aether Files** (`AppKind::Files` in `kernel/src/gui/mod.rs`) is AetherOS's native graphical file manager. It operates directly on the kernel Virtual Filesystem (`fs::vfs_list_dir_snapshot`, `fs::vfs_get_node_clone`, `fs::vfs_read_host_file`, `fs::vfs_write_host_file`, `fs::vfs_mkdir`) and the removable USB Mass Storage layer (`storage::safely_eject_usb_drive`, `storage::remount_or_reinsert_usb_drive`, `storage::sync_usb_fat32_from_vfs`).

## 2. Layout & Interactive Controls

1. **Top Action Toolbar**:
   - `[Up ..]`: Navigates to the parent directory (`fs::parent_and_name`).
   - `[+Folder]`: Creates a new directory (`Folder_N`, mode `0755`) in the current directory.
   - `[+File]`: Creates a new file (`NOTE_N.TXT`, mode `0644`) in the current directory and automatically flushes it to the FAT32 cluster chain via SCSI `WRITE(10)` + `SYNCHRONIZE CACHE(10)` if created inside `/mnt/usb/AETHER_USB`.
   - `[View:List]` / `[View:Grid]`: Toggles between detailed list view (showing type, name, and byte size) and 3-column icon grid view.
   - `[Eject USB]` / `[Mount USB]`: Safely flushes and unmounts `/mnt/usb/AETHER_USB` or remounts `/dev/sdc1`.
   - **Breadcrumb Path Bar**: Displays the active canonical directory path.
2. **Left Locations Sidebar**:
   - One-click navigation to `USB Drive` (`/mnt/usb/AETHER_USB`), `AetherFS` (`/mnt/aether`), `Root /` (`/`), `Home` (`/home/aether`), `Downloads` (`/home/aether/Downloads`), `Config /etc` (`/etc`), `Bins /bin` (`/bin`), and `Temp /tmp` (`/tmp`).
3. **Right UUID & ACL Inspector Panel**:
   - Clicking any file displays its live inode metadata: object name, octal permission mode (`0644`, `0755`, etc.), numeric UID, 128-bit `OwnerUUID`, 128-bit `GroupUUID`, `ACL Count`, and a multi-line live text content preview.
